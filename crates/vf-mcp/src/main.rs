use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use vf_apps::agent::{inspect_module, run_trust_audit, scaffold_vfc_contract, TrustAuditConfig};
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_apps::fault::{run_fault_analysis, FaultConfig, FaultModel, FaultStatus};
use vf_apps::power::{parse_upf_file, run_powercheck, PowerCheckConfig, PowerStatus};
use vf_dsl::{lower_contract_to_ir, parse_contract, ContractRole};
use vf_engine::backtrace::{compute_cex_backtrace, format_agent_trace_table};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::deadend::{check_deadends, DeadEndConfig, DeadEndOutcome};
use vf_engine::equiv::{check_slec, parse_reset_spec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_engine::portfolio::{run_portfolio, PortfolioConfig};
use vf_ir::passes::{async2sync, clk2fflogic, has_multiple_clocks};
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};

const VFC_SPEC_DOC: &str = r#"# VeloFormal Contract Language (.vfc v1.0) — Agent Quick Reference

Write non-invasive formal specifications without modifying production RTL:

```vfc
contract <ContractName> for <rtl_module_name> {
    parameter MAX_VAL: int where MAX_VAL > 0;
    clock posedge clk;
    reset when rst;            // or: reset when !rst_n;

    // Assumptions (environment constraints)
    require valid_input: !enable || unsigned(count) < MAX_VAL;

    // Safety invariants (proved unboundedly via k-induction / IC3-PDR)
    invariant bounded: unsigned(count) <= MAX_VAL;
    invariant stable_idle: !prev(enable) => stable(count);

    // Multi-cycle operations
    operation step when enable {
        ensure advances after 1 cycles:
            count == at_trigger(count) + 1;
    }

    // Reusable protocol templates
    // use handshake(valid, ready, data);
    // use fifo_scoreboard(wr_en, wr_data, rd_en, rd_data, full, empty, 4);

    // Embedded 4-state X-check & IFT security
    // xfree out_data;
    // secure secret_key -> pub_out;

    // Reachability cover goals
    cover reaches_max: unsigned(count) == MAX_VAL;
}
```

Key Rules:
- Always call `inspect_rtl` first to get exact port names, bit widths, clock, and reset polarity.
- Use `unsigned(sig)` or `signed(sig)` when comparing bitvectors against integers or across widths.
- Use `prev(expr)`, `rose(sig)`, `fell(sig)`, `stable(sig)`, `changed(sig)` for 1-cycle history (reset-guarded automatically!).
- Always finish with `audit_trust` to prove zero assumption dead-ends, zero vacuous assertions, and strong mutation kill-rate!
"#;

const WORKFLOWS_DOC: &str = r#"# VeloFormal Human + Agent Formal Verification Workflows

## 1. Verify New or Existing RTL (Spec-Driven Loop)
1. Call `inspect_rtl` on the SystemVerilog file(s) to discover exact ports, widths, clocks, resets, FSMs, and bus protocols.
2. Call `run_autocheck` for instant zero-SVA bug hunting (OOB memory, div-by-zero, shift-overflow, unique/priority, FSM deadlock/livelock/unreachable).
3. Call `scaffold_contract` to generate a starter `.vfc` v1.0 contract, then refine invariants/operations/covers.
4. Call `verify_contract` to prove safety & cover goals. If any assertion fails, read the compact `backtrace` and `trace_table` to diagnose whether the bug is in RTL or the spec.
5. Call `audit_trust` before claiming sign-off: verifies unbounded safety proof + zero assumption dead-ends + non-vacuity + formal mutation kill score (%).

## 2. Safe AI RTL Refactoring / Clock-Gating / Pipelining (SLEC Guardrail)
1. Save or reference the original RTL (`gold_files`) and your optimized/clock-gated/pipelined RTL (`gate_files`).
2. Call `check_equiv` (with `latency_gold`/`latency_gate` for retiming, `reset` for reset init, `valid_cond` for low-power clock gating, `ignore_outputs` for added safety/debug ports).
3. Only accept the refactor when `status == "EQUIVALENT"`.

## 3. Automotive / Safety & Low-Power Sign-Off
- Call `check_fault` with `alarm` for ISO 26262 SA0/SA1/SEU SPFM% & Diagnostic Coverage%.
- Call `check_power` with an IEEE 1801 `.upf` file to formally prove isolation clamp values (`0`, `1`, `latch`) and power-down sequencing.
"#;

fn main() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                writeln!(out, "{err_resp}")?;
                out.flush()?;
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        // JSON-RPC notifications have no `id` and expect no response.
        if id.is_none() {
            continue;
        }
        let id = id.unwrap();

        let resp = match handle_request(method, &params) {
            Ok(result) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": e.to_string() }
            }),
        };
        writeln!(out, "{resp}")?;
        out.flush()?;
    }

    Ok(())
}

fn handle_request(method: &str, params: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {}
            },
            "serverInfo": {
                "name": "veloformal-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("tools/call missing `name`"))?;
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match dispatch_tool(name, &args) {
                Ok(payload) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&payload)?
                    }],
                    "isError": false
                })),
                Err(e) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({ "error": e.to_string() }).to_string()
                    }],
                    "isError": true
                })),
            }
        }
        "resources/list" => Ok(json!({
            "resources": [
                {
                    "uri": "veloformal://vfc-spec",
                    "name": "VeloFormal .vfc v1.0 Contract DSL Reference",
                    "description": "Concise syntax and semantics guide for writing non-invasive .vfc formal contracts",
                    "mimeType": "text/markdown"
                },
                {
                    "uri": "veloformal://workflows",
                    "name": "Human & Agent Formal Verification Workflows",
                    "description": "Step-by-step formal verification, SLEC refactoring, and anti-cheating audit workflows",
                    "mimeType": "text/markdown"
                }
            ]
        })),
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("resources/read missing `uri`"))?;
            let text = match uri {
                "veloformal://vfc-spec" => VFC_SPEC_DOC,
                "veloformal://workflows" => WORKFLOWS_DOC,
                other => bail!("unknown MCP resource URI '{other}'"),
            };
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "text/markdown",
                    "text": text
                }]
            }))
        }
        "prompts/list" => Ok(json!({
            "prompts": [
                {
                    "name": "formal-verify-rtl",
                    "description": "Inspect RTL, run AutoCheck, write a .vfc contract, prove safety, and pass the 4-gate Trust Audit",
                    "arguments": [{ "name": "rtl_path", "description": "Path to Verilog/SV file", "required": true }]
                },
                {
                    "name": "verify-rtl-refactor",
                    "description": "Mathematically prove an RTL optimization, pipelining, or clock-gating refactor via SLEC",
                    "arguments": [
                        { "name": "gold_path", "description": "Reference RTL path", "required": true },
                        { "name": "gate_path", "description": "Refactored RTL path", "required": true }
                    ]
                }
            ]
        })),
        "prompts/get" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("prompts/get missing `name`"))?;
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let prompt_text = match name {
                "formal-verify-rtl" => {
                    let rtl = args
                        .get("rtl_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<rtl.sv>");
                    format!(
                        "Formally verify `{rtl}` using VeloFormal:\n\
                         1. Call `inspect_rtl` on `{rtl}` to inspect exact ports, widths, clock, reset, and FSM candidates.\n\
                         2. Call `run_autocheck` on `{rtl}` to hunt zero-SVA RTL bugs.\n\
                         3. Call `scaffold_contract` and refine a strong `.vfc` contract (check `veloformal://vfc-spec`).\n\
                         4. Call `verify_contract` and fix any counterexample using the COI `backtrace` & `trace_table`.\n\
                         5. Finish by calling `audit_trust` and ensure `trustworthy == true` (unbounded proof + no dead-ends + non-vacuous + strong mutation kill-rate)."
                    )
                }
                "verify-rtl-refactor" => {
                    let gold = args
                        .get("gold_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<gold.sv>");
                    let gate = args
                        .get("gate_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<gate.sv>");
                    format!(
                        "Verify sequential equivalence between reference `{gold}` and refactored `{gate}`:\n\
                         1. Call `inspect_rtl` on both files to check port alignment and reset polarity.\n\
                         2. Call `check_equiv` with appropriate `reset`, `latency_gold`/`latency_gate`, `valid_cond` (for clock gating), or `ignore_outputs`.\n\
                         3. If `NOT EQUIVALENT`, inspect the `trace_table` and repair `{gate}` until `EQUIVALENT`."
                    )
                }
                other => bail!("unknown prompt '{other}'"),
            };
            Ok(json!({
                "messages": [{
                    "role": "user",
                    "content": { "type": "text", "text": prompt_text }
                }]
            }))
        }
        other => bail!("unsupported MCP method '{other}'"),
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "inspect_rtl",
            "description": "Elaborate Verilog/SystemVerilog via Slang and return exact module ports, widths, signedness, clock/reset polarity, registers, memories, FSM candidates, detected bus protocols, and recommended checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Verilog/SV source file paths" },
                    "top": { "type": "string", "description": "Optional top-level module name" },
                    "params": { "type": "array", "items": { "type": "string" }, "description": "Optional parameter overrides like PARAM=VAL" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "scaffold_contract",
            "description": "Synthesize a valid, canonical .vfc v1.0 starter contract directly from elaborated Verilog/SV RTL.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "top": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["files"]
            }
        },
        {
            "name": "verify_contract",
            "description": "Formally verify Verilog/SV RTL against a .vfc v1.0 contract (inline contract_text or contract_file). Returns proof status, cover hits, and compact COI counterexample backtrace + Markdown cycle table on failure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "contract_file": { "type": "string", "description": "Path to .vfc contract file" },
                    "contract_text": { "type": "string", "description": "Inline .vfc contract source (alternative to contract_file)" },
                    "top": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "string" } },
                    "engine": { "type": "string", "enum": ["auto", "portfolio", "kind", "pdr"] },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "prove_sv",
            "description": "Run unbounded formal proof (portfolio k-induction + ABC PDR) on SystemVerilog/SVA assertions, returning structured verdict and compact COI counterexample backtrace on failure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "top": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "string" } },
                    "engine": { "type": "string", "enum": ["auto", "portfolio", "kind", "pdr"] },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "run_autocheck",
            "description": "Push-button zero-SVA formal RTL bug hunting (OOB memory & dynamic bit/part-select, div-by-zero, shift-overflow, unique/priority, formal inferred latches, comb loops, multi-drivers, signed/unsigned/mul overflow, formal width truncation, FSM deadlock/livelock/unreachable with auto-FSM, clock-as-data, unresettable/const/unread registers).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "top": { "type": "string" },
                    "fsm_signals": { "type": "array", "items": { "type": "string" } },
                    "auto_fsm": { "type": "boolean" },
                    "check_overflow": { "type": "boolean" },
                    "check_truncation": { "type": "boolean" },
                    "check_const_regs": { "type": "boolean" },
                    "check_no_reset": { "type": "boolean" },
                    "check_unread": { "type": "boolean" },
                    "strict": { "type": "boolean" },
                    "enable": { "type": "array", "items": { "type": "string" } },
                    "disable": { "type": "array", "items": { "type": "string" } },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "check_equiv",
            "description": "Sequential Logic Equivalence Checking (Questa SLEC): mathematically prove refactored, pipelined, safety-hardened, or clock-gated gate RTL matches reference gold RTL.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "gold_files": { "type": "array", "items": { "type": "string" } },
                    "gate_files": { "type": "array", "items": { "type": "string" } },
                    "latency_gold": { "type": "integer" },
                    "latency_gate": { "type": "integer" },
                    "skip_cycles": { "type": "integer" },
                    "reset": { "type": "string", "description": "Reset sequence like rst=1 or rst_n=0" },
                    "reset_cycles": { "type": "integer" },
                    "valid_cond": { "type": "string", "description": "Low-Power Clock Gating (LPGC) observability qualifier" },
                    "ignore_outputs": { "type": "array", "items": { "type": "string" } },
                    "depth": { "type": "integer" }
                },
                "required": ["gold_files", "gate_files"]
            }
        },
        {
            "name": "check_deadend",
            "description": "Detect over-constrained `assume` statements that drive RTL into a reachable dead-end state where no next-cycle input satisfies assumptions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "top": { "type": "string" },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "check_fault",
            "description": "ISO 26262 Formal Fault Analysis (SA0, SA1, SEU): classify faults as Safe/Masked, Detected by safety alarm, or Dangerous Single-Point Fault (SPF) with SPFM% and Diagnostic Coverage%.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "top": { "type": "string" },
                    "targets": { "type": "array", "items": { "type": "string" } },
                    "outputs": { "type": "array", "items": { "type": "string" } },
                    "alarm": { "type": "string" },
                    "detect_latency": { "type": "integer" },
                    "models": { "type": "array", "items": { "type": "string" } },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "check_power",
            "description": "IEEE 1801 UPF Low-Power Verification: formally prove isolation clamp values (0, 1, latch), power-off sequencing, and detect un-isolated domain crossings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "upf_file": { "type": "string" },
                    "pd_off": { "type": "array", "items": { "type": "string" }, "description": "DOMAIN=signal pairs" },
                    "depth": { "type": "integer" }
                },
                "required": ["files", "upf_file"]
            }
        },
        {
            "name": "audit_trust",
            "description": "4-Gate Anti-Cheating Formal Trustworthiness Audit: verifies unbounded safety proof + zero assumption dead-ends + non-vacuity + formal mutation kill score (%). Returns exact surviving mutants when spec coverage is weak.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" } },
                    "contract_file": { "type": "string" },
                    "contract_text": { "type": "string" },
                    "top": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "string" } },
                    "min_mutation_score": { "type": "number" },
                    "max_mutants": { "type": "integer" },
                    "depth": { "type": "integer" }
                },
                "required": ["files"]
            }
        }
    ])
}

fn dispatch_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "inspect_rtl" => {
            let m = compile_from_args(args, "files")?;
            let info = inspect_module(&m);
            Ok(serde_json::to_value(info)?)
        }
        "scaffold_contract" => {
            let m = compile_from_args(args, "files")?;
            let vfc = scaffold_vfc_contract(&m);
            Ok(json!({
                "module_name": m.name,
                "contract_vfc": vfc
            }))
        }
        "verify_contract" => {
            let base = compile_from_args(args, "files")?;
            let contract_src = load_contract_source(args)?;
            let contract = parse_contract(&contract_src)?;
            let param_map = parse_param_map(args);
            let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &param_map)?;
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(15) as usize;
            let engine = args.get("engine").and_then(|v| v.as_str()).unwrap_or("portfolio");

            let outcome = prove_with_engine(&lowered.module, engine, depth)?;
            let (status, detail, backtrace, trace_table) = match outcome {
                ProveOutcome::Proved { k } => ("PROVED", format!("unbounded proof (k={k})"), None, None),
                ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                } => {
                    let bt = compute_cex_backtrace(&lowered.module, Some(&property), &trace);
                    let tbl = format_agent_trace_table(&lowered.module, Some(&property), &trace, 12);
                    (
                        "FAIL",
                        format!("property `{property}` violated at cycle {step}"),
                        Some(bt),
                        Some(tbl),
                    )
                }
                ProveOutcome::AssumptionsUnsat { step } => (
                    "ASSUMPTIONS_UNSAT",
                    format!("assumptions contradictory at cycle {step}"),
                    None,
                    None,
                ),
                ProveOutcome::Unknown { max_k } => (
                    "UNKNOWN",
                    format!("inconclusive up to k={max_k}"),
                    None,
                    None,
                ),
            };

            let covers: Vec<Value> = if lowered
                .module
                .properties
                .iter()
                .any(|p| p.kind == vf_ir::PropertyKind::Cover)
            {
                run_cover(
                    &lowered.module,
                    &CoverConfig {
                        depth,
                        solver: "z3".into(),
                        append: 0,
                    },
                )?
                .into_iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "reached": c.reached.is_some(),
                        "step": c.reached.map(|(s, _)| s)
                    })
                })
                .collect()
            } else {
                Vec::new()
            };

            Ok(json!({
                "contract": contract.name,
                "module": lowered.module.name,
                "status": status,
                "detail": detail,
                "covers": covers,
                "backtrace": backtrace,
                "trace_table": trace_table
            }))
        }
        "prove_sv" => {
            let m = compile_from_args(args, "files")?;
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let engine = args.get("engine").and_then(|v| v.as_str()).unwrap_or("portfolio");
            let outcome = prove_with_engine(&m, engine, depth)?;
            match outcome {
                ProveOutcome::Proved { k } => Ok(json!({
                    "module": m.name,
                    "status": "PROVED",
                    "k": k
                })),
                ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                } => Ok(json!({
                    "module": m.name,
                    "status": "FAIL",
                    "property": property,
                    "step": step,
                    "backtrace": compute_cex_backtrace(&m, Some(&property), &trace),
                    "trace_table": format_agent_trace_table(&m, Some(&property), &trace, 12)
                })),
                ProveOutcome::AssumptionsUnsat { step } => Ok(json!({
                    "module": m.name,
                    "status": "ASSUMPTIONS_UNSAT",
                    "step": step
                })),
                ProveOutcome::Unknown { max_k } => Ok(json!({
                    "module": m.name,
                    "status": "UNKNOWN",
                    "max_k": max_k
                })),
            }
        }
        "run_autocheck" => {
            let opts = compile_opts_from_args(args, "files")?;
            let strict = args.get("strict").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut cfg = if strict {
                AutoCheckConfig::strict()
            } else {
                AutoCheckConfig::default()
            };
            cfg.fsm_signals = str_array(args, "fsm_signals");
            cfg.auto_fsm |= args.get("auto_fsm").and_then(|v| v.as_bool()).unwrap_or(false);
            cfg.check_overflow |= args
                .get("check_overflow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            cfg.check_truncation |= args
                .get("check_truncation")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            cfg.check_const_regs |= args
                .get("check_const_regs")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            cfg.check_no_reset |= args
                .get("check_no_reset")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            cfg.check_unread |= args
                .get("check_unread")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            cfg.enabled_categories = str_array(args, "enable");
            cfg.disabled_categories = str_array(args, "disable");
            cfg.depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(15) as usize;

            let (m, findings) = run_autocheck(&opts, &cfg)?;
            let mut bugs = 0usize;
            let mut warnings = 0usize;
            let items: Vec<Value> = findings
                .into_iter()
                .map(|f| match f.status {
                    CheckStatus::Safe => json!({
                        "category": f.category,
                        "name": f.name,
                        "status": "SAFE",
                        "detail": f.detail
                    }),
                    CheckStatus::Bug { step, trace } => {
                        bugs += 1;
                        let trace_table = if trace.is_empty() {
                            Value::Null
                        } else {
                            json!(format_agent_trace_table(&m, None, &trace, 10))
                        };
                        json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "BUG",
                            "step": step,
                            "detail": f.detail,
                            "trace_table": trace_table
                        })
                    }
                    CheckStatus::Warning(msg) => {
                        warnings += 1;
                        json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "WARN",
                            "message": msg
                        })
                    }
                    CheckStatus::Unknown => json!({
                        "category": f.category,
                        "name": f.name,
                        "status": "UNKNOWN"
                    }),
                })
                .collect();
            Ok(json!({
                "module": m.name,
                "bugs": bugs,
                "warnings": warnings,
                "findings": items
            }))
        }
        "check_equiv" => {
            let gold = compile_from_args(args, "gold_files")?;
            let gate = compile_from_args(args, "gate_files")?;
            let reset = args
                .get("reset")
                .and_then(|v| v.as_str())
                .map(parse_reset_spec)
                .transpose()?;
            let cfg = SlecConfig {
                depth: args.get("depth").and_then(|v| v.as_u64()).unwrap_or(20) as usize,
                solver: "z3".into(),
                engine: "auto".into(),
                latency_gold: args
                    .get("latency_gold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                latency_gate: args
                    .get("latency_gate")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                skip_cycles: args
                    .get("skip_cycles")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                reset,
                reset_cycles: args
                    .get("reset_cycles")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize,
                outputs: str_array(args, "outputs"),
                ignore_outputs: str_array(args, "ignore_outputs"),
                valid_cond: args
                    .get("valid_cond")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            };
            let (miter, outcome) = check_slec(&gold, &gate, &cfg)?;
            match outcome {
                ProveOutcome::Proved { k } => Ok(json!({
                    "status": "EQUIVALENT",
                    "k": k
                })),
                ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                } => Ok(json!({
                    "status": "NOT_EQUIVALENT",
                    "property": property,
                    "step": step,
                    "trace_table": format_agent_trace_table(&miter, Some(&property), &trace, 12)
                })),
                ProveOutcome::AssumptionsUnsat { step } => Ok(json!({
                    "status": "ASSUMPTIONS_UNSAT",
                    "step": step
                })),
                ProveOutcome::Unknown { max_k } => Ok(json!({
                    "status": "UNKNOWN",
                    "max_k": max_k
                })),
            }
        }
        "check_deadend" => {
            let m = compile_from_args(args, "files")?;
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(15) as usize;
            match check_deadends(&m, &DeadEndConfig { depth, solver: "z3".into() })? {
                DeadEndOutcome::NoAssumptions => Ok(json!({ "status": "CLEAN", "detail": "no assumptions" })),
                DeadEndOutcome::Clean { depth } => Ok(json!({ "status": "CLEAN", "depth": depth })),
                DeadEndOutcome::InitialUnsat => Ok(json!({ "status": "DEAD_END", "step": 0, "detail": "initial state UNSAT" })),
                DeadEndOutcome::DeadEndFound { step, trace } => Ok(json!({
                    "status": "DEAD_END",
                    "step": step,
                    "backtrace": compute_cex_backtrace(&m, None, &trace),
                    "trace_table": format_agent_trace_table(&m, None, &trace, 10)
                })),
                DeadEndOutcome::Unknown { step } => Ok(json!({ "status": "UNKNOWN", "step": step })),
            }
        }
        "check_fault" => {
            let m = compile_from_args(args, "files")?;
            let models: Vec<FaultModel> = {
                let raw = str_array(args, "models");
                if raw.is_empty() {
                    vec![FaultModel::Sa0, FaultModel::Sa1, FaultModel::Seu]
                } else {
                    raw.iter()
                        .map(|s| FaultModel::parse(s))
                        .collect::<Result<_>>()?
                }
            };
            let rep = run_fault_analysis(
                &m,
                &FaultConfig {
                    targets: str_array(args, "targets"),
                    outputs: str_array(args, "outputs"),
                    alarm: args.get("alarm").and_then(|v| v.as_str()).map(String::from),
                    detect_latency: args
                        .get("detect_latency")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    models,
                    depth: args.get("depth").and_then(|v| v.as_u64()).unwrap_or(12) as usize,
                    ..Default::default()
                },
            )?;
            let findings: Vec<Value> = rep
                .findings
                .iter()
                .map(|f| {
                    let status = match &f.status {
                        FaultStatus::SafeMasked => "SAFE/MASKED",
                        FaultStatus::Detected => "DETECTED",
                        FaultStatus::DangerousSpf { .. } => "SPF",
                        FaultStatus::Unknown => "UNKNOWN",
                    };
                    json!({
                        "signal": f.signal,
                        "bit": f.bit,
                        "model": f.model.as_str(),
                        "status": status
                    })
                })
                .collect();
            Ok(json!({
                "module": m.name,
                "total_faults": rep.total_faults,
                "safe_masked": rep.safe_masked,
                "detected": rep.detected,
                "dangerous_spf": rep.dangerous_spf,
                "diagnostic_coverage_pct": rep.diagnostic_coverage_pct,
                "spfm_pct": rep.spfm_pct,
                "findings": findings
            }))
        }
        "check_power" => {
            let m = compile_from_args(args, "files")?;
            let upf_path = args
                .get("upf_file")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("check_power requires `upf_file`"))?;
            let upf = parse_upf_file(&PathBuf::from(upf_path))?;
            let mut pd_off = Vec::new();
            for pair in str_array(args, "pd_off") {
                if let Some((d, s)) = pair.split_once('=') {
                    pd_off.push((d.trim().to_string(), s.trim().to_string()));
                }
            }
            let findings = run_powercheck(
                &m,
                &upf,
                &PowerCheckConfig {
                    pd_off,
                    depth: args.get("depth").and_then(|v| v.as_u64()).unwrap_or(15) as usize,
                    solver: "z3".into(),
                    use_pdr: true,
                },
            )?;
            let items: Vec<Value> = findings
                .into_iter()
                .map(|f| match f.status {
                    PowerStatus::Pass => json!({
                        "domain": f.domain,
                        "signal": f.signal,
                        "check": f.check,
                        "status": "PASS"
                    }),
                    PowerStatus::Fail { step, trace } => json!({
                        "domain": f.domain,
                        "signal": f.signal,
                        "check": f.check,
                        "status": "FAIL",
                        "step": step,
                        "trace_table": format_agent_trace_table(&m, None, &trace, 10)
                    }),
                    PowerStatus::StructuralViolation(msg) => json!({
                        "domain": f.domain,
                        "signal": f.signal,
                        "check": f.check,
                        "status": "VIOLATION",
                        "message": msg
                    }),
                    PowerStatus::Unknown => json!({
                        "domain": f.domain,
                        "signal": f.signal,
                        "check": f.check,
                        "status": "UNKNOWN"
                    }),
                })
                .collect();
            Ok(json!({ "module": m.name, "findings": items }))
        }
        "audit_trust" => {
            let mut m = compile_from_args(args, "files")?;
            if args.get("contract_file").is_some() || args.get("contract_text").is_some() {
                let src = load_contract_source(args)?;
                let contract = parse_contract(&src)?;
                let param_map = parse_param_map(args);
                m = lower_contract_to_ir(&m, &contract, ContractRole::Provider, &param_map)?.module;
            }
            let report = run_trust_audit(
                &m,
                &TrustAuditConfig {
                    depth: args.get("depth").and_then(|v| v.as_u64()).unwrap_or(15) as usize,
                    max_mutants: args
                        .get("max_mutants")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(16) as usize,
                    min_mutation_score: args
                        .get("min_mutation_score")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(50.0),
                    solver: "z3".into(),
                },
            )?;
            Ok(serde_json::to_value(report)?)
        }
        other => bail!("unknown MCP tool '{other}'"),
    }
}

fn prove_with_engine(m: &Module, engine: &str, depth: usize) -> Result<ProveOutcome> {
    match engine {
        "kind" => run_kinduction(
            m,
            &ProveConfig {
                max_k: depth,
                solver: "z3".into(),
            },
        ),
        "pdr" => run_pdr(m, &PdrConfig::default()),
        _ => Ok(run_portfolio(
            m,
            &PortfolioConfig {
                depth,
                solver: "z3".into(),
                ..Default::default()
            },
        )?
        .outcome),
    }
}

fn load_contract_source(args: &Value) -> Result<String> {
    if let Some(text) = args.get("contract_text").and_then(|v| v.as_str()) {
        return Ok(text.to_string());
    }
    if let Some(path) = args.get("contract_file").and_then(|v| v.as_str()) {
        return Ok(fs::read_to_string(path)?);
    }
    bail!("expected either `contract_file` or `contract_text`")
}

fn str_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_param_map(args: &Value) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for p in str_array(args, "params") {
        if let Some((k, v)) = p.split_once('=') {
            if let Ok(n) = v.trim().parse::<i64>() {
                map.insert(k.trim().to_string(), n);
            }
        }
    }
    map
}

fn compile_opts_from_args(args: &Value, files_key: &str) -> Result<CompileOptions> {
    let files: Vec<PathBuf> = str_array(args, files_key)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if files.is_empty() {
        bail!("missing non-empty `{files_key}` file list");
    }
    let mut opts = CompileOptions::for_files(files);
    opts.top = args.get("top").and_then(|v| v.as_str()).map(String::from);
    opts.params = str_array(args, "params");
    Ok(opts)
}

fn compile_from_args(args: &Value, files_key: &str) -> Result<Module> {
    let opts = compile_opts_from_args(args, files_key)?;
    let mut m = compile_to_ir(&opts)?;
    if has_multiple_clocks(&m) {
        clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        async2sync(&mut m);
    }
    Ok(m)
}
