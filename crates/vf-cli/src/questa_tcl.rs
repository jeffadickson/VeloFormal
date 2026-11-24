use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_apps::cdc::{parse_sdc_file, run_cdc_analysis, CdcConfig, CdcSeverity, GrayStatus};
use vf_apps::covercheck::{run_covercheck, CoverCheckConfig, CoverageStatus};
use vf_apps::fault::{run_fault_analysis, FaultConfig, FaultStatus};
use vf_apps::power::{parse_upf_file, run_powercheck, PowerCheckConfig, PowerStatus};
use vf_apps::regcheck::{
    parse_regmap_auto, run_regcheck, RegBusConfig, RegCheckStatus, RegProtocol,
};
use vf_apps::xcheck::{run_xcheck, XCheckConfig, XStatus};
use vf_dsl::{lower_contract_to_ir, parse_contract, ContractRole};
use vf_engine::deadend::{check_deadends, DeadEndConfig, DeadEndOutcome};
use vf_engine::equiv::{check_slec, parse_reset_spec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::passes::{async2sync, clk2fflogic, has_multiple_clocks};
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};

use crate::report::{ReportItem, SignOffReport};

/// Execute a Questa-Formal-compatible `.do` / Tcl verification script.
///
/// Supported commands:
/// - `formal compile -d <top> <files...>` / `vlog <files...>`
/// - `netlist clock <sig> [-period <ns>]`
/// - `netlist reset <sig> [-active_low | -active_high]`
/// - `formal verify [-depth <N>] [-engine auto|kind|pdr]`
/// - `formal contract <spec.vfc> [-depth <N>] [-engine auto|kind|pdr]`
/// - `autocheck verify [-depth <N>] [-fsm <sig>]`
/// - `covercheck verify [-depth <N>]`
/// - `cdc run [-sdc <file.sdc>] [-gray <sig>]`
/// - `xcheck verify [-target <sig>] [-skip <N>]`
/// - `regcheck verify -spec <file.rdl|file.xml|file.csv>`
/// - `slec verify -gold <file> -gate <file> [-latency_gold <N>] [-latency_gate <N>]`
pub fn run_questa_tcl(script_path: &Path) -> Result<SignOffReport> {
    let base_dir = script_path.parent().unwrap_or_else(|| Path::new("."));
    let text = fs::read_to_string(script_path)
        .with_context(|| format!("failed to read Tcl script {}", script_path.display()))?;

    let mut files: Vec<PathBuf> = Vec::new();
    let mut top: Option<String> = None;
    let mut reset_sig: Option<String> = None;
    let mut reset_active_low: Option<bool> = None;
    let mut report = SignOffReport::new(
        format!("Questa Tcl: {}", script_path.display()),
        "unspecified",
    );

    let resolve = |p: &str| -> PathBuf {
        let pb = PathBuf::from(p);
        if pb.is_absolute() {
            pb
        } else {
            base_dir.join(pb)
        }
    };

    let compile_raw = |files: &[PathBuf], top: &Option<String>| -> Result<Module> {
        if files.is_empty() {
            bail!("no Verilog/SV files compiled yet (use `formal compile -d <top> <files...>`)");
        }
        let mut opts = CompileOptions::for_files(files.to_vec());
        opts.top = top.clone();
        compile_to_ir(&opts)
    };

    let prepare = |mut m: Module| -> Module {
        if has_multiple_clocks(&m) {
            clk2fflogic(&mut m);
        }
        if m.ffs.iter().any(|f| f.async_reset.is_some()) {
            async2sync(&mut m);
        }
        m
    };

    for (line_no, raw_line) in text.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks = split_tcl_tokens(line);
        if toks.is_empty() {
            continue;
        }
        match toks[0].as_str() {
            "vlog" | "read_verilog" => {
                for t in &toks[1..] {
                    if !t.starts_with('-') && !t.starts_with('+') {
                        files.push(resolve(t));
                    }
                }
            }
            "netlist" => match toks.get(1).map(String::as_str) {
                Some("clock") => {
                    // Recognized clock directive (Slang extracts clocks from always_ff sensitivity lists).
                }
                Some("reset") => {
                    if let Some(sig) = toks.get(2) {
                        reset_sig = Some(sig.clone());
                    }
                    if toks.iter().any(|t| t == "-active_low") {
                        reset_active_low = Some(true);
                    } else if toks.iter().any(|t| t == "-active_high") {
                        reset_active_low = Some(false);
                    }
                }
                _ => {}
            },
            "formal" => match toks.get(1).map(String::as_str) {
                Some("compile") => {
                    let mut i = 2;
                    while i < toks.len() {
                        if (toks[i] == "-d" || toks[i] == "-top") && i + 1 < toks.len() {
                            top = Some(toks[i + 1].clone());
                            i += 2;
                        } else if toks[i].starts_with('-') {
                            i += 1;
                        } else {
                            files.push(resolve(&toks[i]));
                            i += 1;
                        }
                    }
                    if let Some(t) = &top {
                        report.module_name = t.clone();
                    }
                }
                Some("verify") => {
                    let depth = flag_usize(&toks, "-depth").unwrap_or(15);
                    let engine = flag_str(&toks, "-engine").unwrap_or("auto");
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let outcome = prove_module(&m, depth, engine)?;
                    push_prove_outcome(&mut report, "formal_verify", engine, outcome);
                }
                Some("contract") => {
                    let spec_p = toks
                        .get(2)
                        .ok_or_else(|| anyhow!("line {}: `formal contract` expects `.vfc` file", line_no + 1))?;
                    let depth = flag_usize(&toks, "-depth").unwrap_or(15);
                    let engine = flag_str(&toks, "-engine").unwrap_or("auto");
                    let c_src = fs::read_to_string(resolve(spec_p))?;
                    let contract = parse_contract(&c_src)?;
                    let base = prepare(compile_raw(&files, &top)?);
                    let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &HashMap::new())?;
                    report.module_name = lowered.module.name.clone();
                    let outcome = prove_module(&lowered.module, depth, engine)?;
                    push_prove_outcome(&mut report, &contract.name, engine, outcome);
                }
                other => bail!("line {}: unsupported `formal {:?}` directive", line_no + 1, other),
            },
            "autocheck" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let depth = flag_usize(&toks, "-depth").unwrap_or(15);
                    let fsm = flag_all(&toks, "-fsm");
                    let mut opts = CompileOptions::for_files(files.clone());
                    opts.top = top.clone();
                    let (m, findings) = run_autocheck(
                        &opts,
                        &AutoCheckConfig {
                            depth,
                            fsm_signals: fsm,
                            ..Default::default()
                        },
                    )?;
                    report.module_name = m.name.clone();
                    for f in findings {
                        let (status, detail, trace) = match f.status {
                            CheckStatus::Safe => ("SAFE".into(), "proved safe".into(), None),
                            CheckStatus::Bug { step, trace } => {
                                ("BUG".into(), format!("violated at cycle {step}"), Some(trace))
                            }
                            CheckStatus::Warning(msg) => ("WARN".into(), msg, None),
                            CheckStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: f.name,
                            category: format!("autocheck:{}", f.category),
                            status,
                            engine: "autocheck".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "covercheck" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let depth = flag_usize(&toks, "-depth").unwrap_or(15);
                    let mut opts = CompileOptions::for_files(files.clone());
                    opts.top = top.clone();
                    let (m, items) = run_covercheck(
                        &opts,
                        &CoverCheckConfig {
                            depth,
                            ..Default::default()
                        },
                    )?;
                    report.module_name = m.name.clone();
                    for item in items {
                        let (status, detail, trace) = match item.status {
                            CoverageStatus::Reachable { step, trace } => {
                                ("REACHED".into(), format!("reached at cycle {step}"), Some(trace))
                            }
                            CoverageStatus::Unreachable => {
                                ("UNREACHABLE".into(), "proved dead code".into(), None)
                            }
                            CoverageStatus::Undecided => ("UNKNOWN".into(), "undecided".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: item.name,
                            category: "covercheck".into(),
                            status,
                            engine: "covercheck".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "cdc" => {
                if toks.get(1).map(String::as_str) == Some("run") {
                    let gray = flag_all(&toks, "-gray");
                    let sdc = flag_str(&toks, "-sdc")
                        .map(|p| parse_sdc_file(&resolve(p)))
                        .transpose()?;
                    let raw = compile_raw(&files, &top)?;
                    report.module_name = raw.name.clone();
                    let res = run_cdc_analysis(
                        &raw,
                        &CdcConfig {
                            gray_signals: gray,
                            sdc,
                            ..Default::default()
                        },
                    )?;
                    for f in res.findings {
                        let status = match f.severity {
                            CdcSeverity::Error => "ERROR",
                            CdcSeverity::Warning => "WARN",
                            CdcSeverity::Safe => "SAFE",
                        };
                        report.items.push(ReportItem {
                            name: format!("{} -> {}", f.src_signal, f.dst_signal),
                            category: format!("cdc:{}", f.kind),
                            status: status.into(),
                            engine: "cdc".into(),
                            detail: f.detail,
                            source: None,
                            trace: None,
                        });
                    }
                    for g in res.gray_results {
                        let (status, detail, trace) = match g.status {
                            GrayStatus::Proved => ("PROVED".into(), "Gray-code invariant holds".into(), None),
                            GrayStatus::Fail { step, trace } => {
                                ("FAIL".into(), format!("changed >1 bit at cycle {step}"), Some(trace))
                            }
                            GrayStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: g.signal,
                            category: "cdc:gray".into(),
                            status,
                            engine: "cdc".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "xcheck" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let targets = flag_all(&toks, "-target");
                    let skip_cycles = flag_usize(&toks, "-skip").unwrap_or(0);
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let (_, results) = run_xcheck(
                        &m,
                        &XCheckConfig {
                            targets,
                            skip_cycles,
                            ..Default::default()
                        },
                    )?;
                    for r in results {
                        let (status, detail, trace) = match r.status {
                            XStatus::XFree => ("X-FREE".into(), "proved 4-state X-free".into(), None),
                            XStatus::XLeak { step, trace } => {
                                ("X-LEAK".into(), format!("X propagated at cycle {step}"), Some(trace))
                            }
                            XStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: r.signal,
                            category: "xcheck".into(),
                            status,
                            engine: "xcheck".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "regcheck" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let spec_p = flag_str(&toks, "-spec")
                        .ok_or_else(|| anyhow!("line {}: `regcheck verify` requires `-spec <file>`", line_no + 1))?;
                    let proto = flag_str(&toks, "-protocol")
                        .map(RegProtocol::parse)
                        .transpose()?
                        .unwrap_or_default();
                    let read_latency = flag_usize(&toks, "-read_latency").unwrap_or(0);
                    let specs = parse_regmap_auto(&resolve(spec_p))?;
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let findings = run_regcheck(
                        &m,
                        &specs,
                        &RegBusConfig {
                            protocol: proto,
                            read_latency,
                            rst: reset_sig.clone(),
                            rst_active_low: reset_active_low,
                            depth: 10,
                            solver: "z3".into(),
                            use_pdr: true,
                            ..Default::default()
                        },
                    )?;
                    for f in findings {
                        let (status, detail, trace) = match f.status {
                            RegCheckStatus::Pass => ("PASS".into(), "register check proved".into(), None),
                            RegCheckStatus::Fail { step, trace } => {
                                ("FAIL".into(), format!("violated at cycle {step}"), Some(trace))
                            }
                            RegCheckStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: format!("{}.{}", f.register, f.check),
                            category: "regcheck".into(),
                            status,
                            engine: "regcheck".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "slec" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let gold_p = flag_str(&toks, "-gold")
                        .ok_or_else(|| anyhow!("line {}: `slec verify` requires `-gold <file>`", line_no + 1))?;
                    let gate_p = flag_str(&toks, "-gate")
                        .ok_or_else(|| anyhow!("line {}: `slec verify` requires `-gate <file>`", line_no + 1))?;
                    let lat_gold = flag_usize(&toks, "-latency_gold").unwrap_or(0);
                    let lat_gate = flag_usize(&toks, "-latency_gate").unwrap_or(0);
                    let reset = flag_str(&toks, "-reset").map(parse_reset_spec).transpose()?;
                    let reset_cycles = flag_usize(&toks, "-reset_cycles").unwrap_or(1);
                    let ignore_outputs = flag_all(&toks, "-ignore_output");
                    let valid_cond = flag_str(&toks, "-valid_cond").map(String::from);
                    let gold = prepare(compile_to_ir(&CompileOptions::for_files(vec![resolve(gold_p)]))?);
                    let gate = prepare(compile_to_ir(&CompileOptions::for_files(vec![resolve(gate_p)]))?);
                    report.module_name = format!("{} vs {}", gold.name, gate.name);
                    let (_, outcome) = check_slec(
                        &gold,
                        &gate,
                        &SlecConfig {
                            latency_gold: lat_gold,
                            latency_gate: lat_gate,
                            reset,
                            reset_cycles,
                            ignore_outputs,
                            valid_cond,
                            ..Default::default()
                        },
                    )?;
                    push_prove_outcome(&mut report, "slec_equiv", "slec", outcome);
                }
            }
            "fault" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let targets = flag_all(&toks, "-target");
                    let outputs = flag_all(&toks, "-output");
                    let alarm = flag_str(&toks, "-alarm").map(String::from);
                    let depth = flag_usize(&toks, "-depth").unwrap_or(12);
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let frep = run_fault_analysis(
                        &m,
                        &FaultConfig {
                            targets,
                            outputs,
                            alarm,
                            depth,
                            ..Default::default()
                        },
                    )?;
                    for f in frep.findings {
                        let fname = format!("{}[{}]/{}", f.signal, f.bit, f.model.as_str());
                        let (status, detail, trace) = match f.status {
                            FaultStatus::SafeMasked => ("SAFE/MASKED".into(), "masked".into(), None),
                            FaultStatus::Detected => ("DETECTED".into(), "covered by alarm".into(), None),
                            FaultStatus::DangerousSpf { step, trace } => {
                                ("SPF".into(), format!("dangerous SPF at cycle {step}"), Some(trace))
                            }
                            FaultStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: fname,
                            category: "fault".into(),
                            status,
                            engine: "fault-miter".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "powercheck" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let upf_p = flag_str(&toks, "-upf")
                        .ok_or_else(|| anyhow!("line {}: `powercheck verify` requires `-upf <file>`", line_no + 1))?;
                    let upf_spec = parse_upf_file(&resolve(upf_p))?;
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let findings = run_powercheck(&m, &upf_spec, &PowerCheckConfig::default())?;
                    for f in findings {
                        let (status, detail, trace) = match f.status {
                            PowerStatus::Pass => ("PASS".into(), format!("{} proved", f.check), None),
                            PowerStatus::Fail { step, trace } => {
                                ("FAIL".into(), format!("{} violated at cycle {step}", f.check), Some(trace))
                            }
                            PowerStatus::StructuralViolation(msg) => ("VIOLATION".into(), msg, None),
                            PowerStatus::Unknown => ("UNKNOWN".into(), "inconclusive".into(), None),
                        };
                        report.items.push(ReportItem {
                            name: format!("{}.{}", f.domain, f.signal),
                            category: "powercheck".into(),
                            status,
                            engine: "powercheck".into(),
                            detail,
                            source: None,
                            trace,
                        });
                    }
                }
            }
            "deadend" => {
                if toks.get(1).map(String::as_str) == Some("verify") {
                    let depth = flag_usize(&toks, "-depth").unwrap_or(15);
                    let m = prepare(compile_raw(&files, &top)?);
                    report.module_name = m.name.clone();
                    let (status, detail, trace) = match check_deadends(&m, &DeadEndConfig { depth, solver: "z3".into() })? {
                        DeadEndOutcome::NoAssumptions => ("CLEAN".into(), "no assumptions".into(), None),
                        DeadEndOutcome::Clean { depth } => ("CLEAN".into(), format!("no dead-end up to {depth}"), None),
                        DeadEndOutcome::InitialUnsat => ("DEAD-END".into(), "initial state UNSAT".into(), None),
                        DeadEndOutcome::DeadEndFound { step, trace } => ("DEAD-END".into(), format!("dead-end at cycle {step}"), Some(trace)),
                        DeadEndOutcome::Unknown { step } => ("UNKNOWN".into(), format!("unknown at {step}"), None),
                    };
                    report.items.push(ReportItem {
                        name: "assumption_deadend".into(),
                        category: "deadend".into(),
                        status,
                        engine: "deadend".into(),
                        detail,
                        source: None,
                        trace,
                    });
                }
            }
            "exit" | "quit" => break,
            other => bail!("line {}: unsupported Questa Tcl command '{other}'", line_no + 1),
        }
    }

    Ok(report)
}

fn prove_module(m: &Module, depth: usize, engine: &str) -> Result<ProveOutcome> {
    match engine {
        "pdr" => run_pdr(m, &PdrConfig::default()),
        "kind" => run_kinduction(
            m,
            &ProveConfig {
                max_k: depth,
                solver: "z3".into(),
            },
        ),
        _ => {
            let k = run_kinduction(
                m,
                &ProveConfig {
                    max_k: depth,
                    solver: "z3".into(),
                },
            )?;
            match k {
                ProveOutcome::Unknown { .. } => {
                    Ok(run_pdr(m, &PdrConfig::default()).unwrap_or(k))
                }
                other => Ok(other),
            }
        }
    }
}

fn push_prove_outcome(
    report: &mut SignOffReport,
    name: &str,
    engine: &str,
    outcome: ProveOutcome,
) {
    match outcome {
        ProveOutcome::Proved { k } => report.items.push(ReportItem {
            name: name.into(),
            category: "formal".into(),
            status: "PROVED".into(),
            engine: engine.into(),
            detail: if k == 0 {
                "unbounded proof (PDR)".into()
            } else {
                format!("unbounded proof (k-induction k={k})")
            },
            source: None,
            trace: None,
        }),
        ProveOutcome::Fail {
            property,
            step,
            trace,
        } => report.items.push(ReportItem {
            name: property,
            category: "formal".into(),
            status: "FAIL".into(),
            engine: engine.into(),
            detail: format!("violated at cycle {step}"),
            source: None,
            trace: Some(trace),
        }),
        ProveOutcome::AssumptionsUnsat { step } => report.items.push(ReportItem {
            name: name.into(),
            category: "formal".into(),
            status: "FAIL".into(),
            engine: engine.into(),
            detail: format!("assumptions unsatisfiable at cycle {step}"),
            source: None,
            trace: None,
        }),
        ProveOutcome::Unknown { max_k } => report.items.push(ReportItem {
            name: name.into(),
            category: "formal".into(),
            status: "UNKNOWN".into(),
            engine: engine.into(),
            detail: format!("not decided up to k={max_k}"),
            source: None,
            trace: None,
        }),
    }
}

fn split_tcl_tokens(line: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in line.chars() {
        if ch == '"' {
            in_quote = !in_quote;
        } else if ch.is_whitespace() && !in_quote {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

fn flag_str<'a>(toks: &'a [String], flag: &str) -> Option<&'a str> {
    toks.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].as_str())
}

fn flag_usize(toks: &[String], flag: &str) -> Option<usize> {
    flag_str(toks, flag).and_then(|s| s.parse().ok())
}

fn flag_all(toks: &[String], flag: &str) -> Vec<String> {
    toks.windows(2)
        .filter(|w| w[0] == flag)
        .map(|w| w[1].clone())
        .collect()
}
