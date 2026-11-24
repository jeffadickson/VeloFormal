use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;
use vf_engine::backtrace::{compute_cex_backtrace, format_agent_trace_table, CexBacktrace};
use vf_engine::coverage::{run_mutation_analysis, MutantStatus, MutationConfig};
use vf_engine::deadend::{check_deadends, DeadEndConfig, DeadEndOutcome};
use vf_engine::kind::ProveOutcome;
use vf_engine::portfolio::{run_portfolio, PortfolioConfig};
use vf_engine::vacuity::{check_vacuity, VacuityConfig, VacuityStatus};
use vf_ir::{ExprKind, Module, PropertyKind, SignalKind};

#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub name: String,
    pub width: u32,
    pub signed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResetInfo {
    pub name: String,
    pub active_low: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegInfo {
    pub name: String,
    pub width: u32,
    pub init: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemInfo {
    pub name: String,
    pub depth: u32,
    pub data_width: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PropCounts {
    pub asserts: usize,
    pub assumes: usize,
    pub covers: usize,
    pub live: usize,
    pub fair: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RtlInspection {
    pub module_name: String,
    pub clocks: Vec<String>,
    pub reset: Option<ResetInfo>,
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<PortInfo>,
    pub registers: Vec<RegInfo>,
    pub memories: Vec<MemInfo>,
    pub fsm_candidates: Vec<String>,
    pub detected_protocols: Vec<String>,
    pub existing_properties: PropCounts,
    pub recommended_checks: Vec<String>,
}

/// Elaborated RTL introspection for both humans and AI agents:
/// extracts exact port widths/signedness, clocks, reset polarity, registers, memories,
/// FSM state candidates, bus protocols, and recommended formal checks.
pub fn inspect_module(m: &Module) -> RtlInspection {
    let mut clocks: Vec<String> = Vec::new();
    let mut clock_set: HashSet<String> = HashSet::new();
    for s in &m.signals {
        if s.is_clock && !s.name.starts_with('$') && clock_set.insert(s.name.clone()) {
            clocks.push(s.name.clone());
        }
    }
    for ff in &m.ffs {
        if let Some(c) = ff.clock {
            let cname = &m.signal(c.signal).name;
            if !cname.starts_with('$') && clock_set.insert(cname.clone()) {
                clocks.push(cname.clone());
            }
        }
    }

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut input_names: HashSet<String> = HashSet::new();
    let mut output_names: HashSet<String> = HashSet::new();
    for s in &m.signals {
        if s.name.starts_with('$') {
            continue;
        }
        match s.kind {
            SignalKind::Input => {
                input_names.insert(s.name.to_ascii_lowercase());
                inputs.push(PortInfo {
                    name: s.name.clone(),
                    width: s.width,
                    signed: s.signed,
                });
            }
            SignalKind::Output => {
                output_names.insert(s.name.to_ascii_lowercase());
                outputs.push(PortInfo {
                    name: s.name.clone(),
                    width: s.width,
                    signed: s.signed,
                });
            }
            _ => {}
        }
    }

    // Auto-detect reset port and active polarity.
    let reset = ["rst", "rst_n", "reset", "reset_n", "presetn", "aresetn"]
        .iter()
        .find_map(|cand| {
            m.signals.iter().find(|s| {
                s.kind == SignalKind::Input
                    && s.width == 1
                    && s.name.eq_ignore_ascii_case(cand)
            })
        })
        .map(|s| {
            let lo = s.name.to_ascii_lowercase();
            let active_low = lo.ends_with("_n") || lo.ends_with("resetn") || lo == "rstn";
            ResetInfo {
                name: s.name.clone(),
                active_low,
            }
        });

    let mut registers = Vec::new();
    let mut fsm_candidates = Vec::new();
    for ff in &m.ffs {
        let s = m.signal(ff.signal);
        if s.name.starts_with('$') || s.is_clock {
            continue;
        }
        let init = ff
            .init
            .as_ref()
            .or(s.init.as_ref())
            .map(|c| format!("{}'b{}", c.width, c.bits));
        registers.push(RegInfo {
            name: s.name.clone(),
            width: s.width,
            init,
        });

        let lo = s.name.to_ascii_lowercase();
        let name_looks_fsm = lo.contains("state") || lo.contains("fsm");
        let cmp_count = if s.width >= 2 && s.width <= 6 {
            m.exprs
                .iter()
                .filter(|n| match &n.kind {
                    ExprKind::Eq(a, b) => matches!(
                        (&m.expr(*a).kind, &m.expr(*b).kind),
                        (ExprKind::Signal(id), ExprKind::Const(_))
                            | (ExprKind::Const(_), ExprKind::Signal(id))
                            if *id == ff.signal
                    ),
                    _ => false,
                })
                .count()
        } else {
            0
        };
        if name_looks_fsm || cmp_count >= 2 {
            fsm_candidates.push(s.name.clone());
        }
    }

    let memories: Vec<MemInfo> = m
        .memories
        .iter()
        .map(|mem| MemInfo {
            name: mem.name.clone(),
            depth: mem.depth,
            data_width: mem.data_width,
        })
        .collect();

    // Protocol heuristic detection.
    let all_ports: HashSet<String> = input_names.union(&output_names).cloned().collect();
    let mut detected_protocols = Vec::new();
    if all_ports.contains("psel") && all_ports.contains("penable") && all_ports.contains("pwrite") {
        detected_protocols.push("apb".into());
    }
    if all_ports.contains("awvalid") && all_ports.contains("wvalid") && all_ports.contains("bvalid")
    {
        detected_protocols.push("axi4_lite".into());
    }
    if all_ports.contains("tvalid") && all_ports.contains("tready") {
        detected_protocols.push("axi4_stream".into());
    }
    if (all_ports.contains("valid") && all_ports.contains("ready"))
        || (all_ports.contains("in_valid") && all_ports.contains("in_ready"))
    {
        detected_protocols.push("ready_valid".into());
    }
    if all_ports.contains("full")
        && all_ports.contains("empty")
        && (all_ports.contains("wr_en") || all_ports.contains("push"))
    {
        detected_protocols.push("fifo".into());
    }
    if all_ports.contains("wr_en") && all_ports.contains("addr") && all_ports.contains("rdata") {
        detected_protocols.push("csr_bus".into());
    }

    let mut existing_properties = PropCounts {
        asserts: 0,
        assumes: 0,
        covers: 0,
        live: 0,
        fair: 0,
    };
    for p in &m.properties {
        match p.kind {
            PropertyKind::Assert => existing_properties.asserts += 1,
            PropertyKind::Assume => existing_properties.assumes += 1,
            PropertyKind::Cover => existing_properties.covers += 1,
            PropertyKind::Live => existing_properties.live += 1,
            PropertyKind::Fair => existing_properties.fair += 1,
        }
    }

    let mut recommended_checks = Vec::new();
    if fsm_candidates.is_empty() {
        recommended_checks.push("veloformal autocheck <rtl.sv>".into());
    } else {
        recommended_checks.push(format!(
            "veloformal autocheck <rtl.sv> --fsm {}",
            fsm_candidates[0]
        ));
    }
    recommended_checks.push("veloformal covercheck <rtl.sv>".into());
    if reset.is_some() && !outputs.is_empty() {
        recommended_checks.push("veloformal xcheck <rtl.sv> --skip-cycles 1".into());
    }
    if detected_protocols.iter().any(|p| p == "apb") {
        recommended_checks.push("veloformal regcheck <rtl.sv> --spec <regs.rdl> --protocol apb".into());
    } else if detected_protocols.iter().any(|p| p == "csr_bus") {
        recommended_checks.push("veloformal regcheck <rtl.sv> --spec <regs.csv>".into());
    }
    if existing_properties.asserts > 0 {
        recommended_checks.push("veloformal audit <rtl.sv>".into());
    } else {
        recommended_checks.push("veloformal scaffold <rtl.sv> -o spec.vfc".into());
    }

    RtlInspection {
        module_name: m.name.clone(),
        clocks,
        reset,
        inputs,
        outputs,
        registers,
        memories,
        fsm_candidates,
        detected_protocols,
        existing_properties,
        recommended_checks,
    }
}

/// Synthesize a valid, canonical `.vfc` v1.0 starter contract skeleton directly from elaborated RTL.
pub fn scaffold_vfc_contract(m: &Module) -> String {
    let info = inspect_module(m);
    let contract_name = to_pascal_case(&info.module_name);
    let mut out = String::new();
    out.push_str(&format!(
        "// Auto-scaffolded VeloFormal Contract (.vfc v1.0) for `{}`\n",
        info.module_name
    ));
    out.push_str(&format!(
        "contract {contract_name}Spec for {} {{\n",
        info.module_name
    ));

    if let Some(clk) = info.clocks.first() {
        out.push_str(&format!("    clock posedge {clk};\n"));
    }
    if let Some(rst) = &info.reset {
        if rst.active_low {
            out.push_str(&format!("    reset when !{};\n", rst.name));
        } else {
            out.push_str(&format!("    reset when {};\n", rst.name));
        }
    }
    out.push('\n');

    // Port summary comments for human + agent reference.
    if !info.inputs.is_empty() {
        let desc: Vec<String> = info
            .inputs
            .iter()
            .map(|p| format!("{}[{}]", p.name, p.width))
            .collect();
        out.push_str(&format!("    // Inputs:  {}\n", desc.join(", ")));
    }
    if !info.outputs.is_empty() {
        let desc: Vec<String> = info
            .outputs
            .iter()
            .map(|p| format!("{}[{}]", p.name, p.width))
            .collect();
        out.push_str(&format!("    // Outputs: {}\n", desc.join(", ")));
    }
    out.push('\n');

    // Protocol-specific starter invariants / templates.
    if info.detected_protocols.iter().any(|p| p == "fifo") {
        out.push_str("    invariant not_full_and_empty: !(full && empty);\n");
        out.push_str("    // use fifo_scoreboard(wr_en, wr_data, rd_en, rd_data, full, empty, 4);\n");
        out.push_str("    cover reaches_full: full;\n");
    } else {
        if info.detected_protocols.iter().any(|p| p == "ready_valid") {
            if info.outputs.iter().any(|p| p.name == "valid")
                && info.inputs.iter().any(|p| p.name == "ready")
            {
                if let Some(data_p) = info
                    .outputs
                    .iter()
                    .find(|p| p.name.contains("data") || p.name == "dout")
                {
                    out.push_str(&format!(
                        "    use handshake(valid, ready, {});\n",
                        data_p.name
                    ));
                }
            }
        }
        // Emit a valid starter safety invariant + reachability cover goal.
        if let Some(first_out) = info.outputs.first() {
            if first_out.width == 1 {
                out.push_str(&format!(
                    "    // TODO: refine safety invariants for `{}`\n",
                    info.module_name
                ));
                out.push_str(&format!(
                    "    invariant output_defined: {} || !{};\n",
                    first_out.name, first_out.name
                ));
                out.push_str(&format!(
                    "    cover output_high: {};\n",
                    first_out.name
                ));
            } else {
                let max_val = if first_out.width >= 63 {
                    u64::MAX
                } else {
                    (1u64 << first_out.width) - 1
                };
                out.push_str(&format!(
                    "    // TODO: tighten safety bounds for `{}`\n",
                    info.module_name
                ));
                out.push_str(&format!(
                    "    invariant output_in_range: unsigned({}) <= {max_val};\n",
                    first_out.name
                ));
                out.push_str(&format!(
                    "    cover output_nonzero: unsigned({}) > 0;\n",
                    first_out.name
                ));
            }
        } else {
            out.push_str("    invariant sanity: true;\n");
        }
    }

    out.push_str("}\n");
    out
}

fn to_pascal_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            cap = true;
        } else if cap {
            out.push(ch.to_ascii_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "Design".into()
    } else {
        out
    }
}

#[derive(Debug, Clone)]
pub struct TrustAuditConfig {
    pub depth: usize,
    pub max_mutants: usize,
    /// Minimum formal mutation kill score (%) required for sign-off (0.0 disables threshold).
    pub min_mutation_score: f64,
    pub solver: String,
}

impl Default for TrustAuditConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            max_mutants: 16,
            min_mutation_score: 50.0,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustAuditReport {
    pub module_name: String,
    /// Overall anti-cheating trustworthiness verdict: `true` iff ALL 4 gates pass!
    pub trustworthy: bool,
    pub assert_count: usize,
    pub assume_count: usize,
    pub proof_status: String,
    pub deadend_status: String,
    pub vacuous_properties: Vec<String>,
    pub mutation_total: usize,
    pub mutation_killed: usize,
    pub mutation_score_pct: f64,
    pub surviving_mutants: Vec<String>,
    pub issues: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_backtrace: Option<CexBacktrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_trace_table: Option<String>,
}

/// Agentic Era Anti-Cheating / Trustworthiness Audit (`veloformal audit`):
/// Runs a 4-gate formal trustworthiness verification to guarantee neither human nor AI agent
/// cheated with vacuous assertions, over-constrained dead-end assumptions, or weak specs:
/// 1. Unbounded Safety Proof (parallel k-ind + ABC PDR portfolio)
/// 2. Over-Constraint / Dead-End Detection (`check_deadends`)
/// 3. Antecedent Vacuity Check (`check_vacuity`)
/// 4. Formal Mutation Testing (`run_mutation_analysis` kill score >= threshold)
pub fn run_trust_audit(m: &Module, cfg: &TrustAuditConfig) -> Result<TrustAuditReport> {
    let assert_count = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assert)
        .count();
    let assume_count = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assume)
        .count();

    let mut issues = Vec::new();
    if assert_count == 0 {
        issues.push("design/contract contains 0 assertions (nothing to prove)".into());
    }

    // Gate 1: Unbounded Safety Proof.
    let mut failure_backtrace = None;
    let mut failure_trace_table = None;
    let (proof_ok, proof_status) = if assert_count == 0 {
        (false, "NO_ASSERTS".to_string())
    } else {
        let pr = run_portfolio(
            m,
            &PortfolioConfig {
                depth: cfg.depth,
                solver: cfg.solver.clone(),
                ..Default::default()
            },
        )?;
        match pr.outcome {
            ProveOutcome::Proved { k } => (true, format!("PROVED (winner={}, k={k})", pr.winner)),
            ProveOutcome::Fail {
                property,
                step,
                trace,
            } => {
                issues.push(format!("assertion `{property}` FAILED at cycle {step}"));
                failure_backtrace = Some(compute_cex_backtrace(m, Some(&property), &trace));
                failure_trace_table = Some(format_agent_trace_table(m, Some(&property), &trace, 12));
                (false, format!("FAIL ({property} @ t={step})"))
            }
            ProveOutcome::AssumptionsUnsat { step } => {
                issues.push(format!(
                    "assumptions are contradictory (UNSAT) at cycle {step}"
                ));
                (false, format!("ASSUMPTIONS_UNSAT (@ t={step})"))
            }
            ProveOutcome::Unknown { max_k } => {
                issues.push(format!("unbounded proof inconclusive up to k={max_k}"));
                (false, format!("UNKNOWN (k={max_k})"))
            }
        }
    };

    // Gate 2: Over-Constraint / Dead-End Detection.
    let (deadend_ok, deadend_status) = match check_deadends(
        m,
        &DeadEndConfig {
            depth: cfg.depth.min(12),
            solver: cfg.solver.clone(),
        },
    )? {
        DeadEndOutcome::NoAssumptions => (true, "CLEAN (no assumptions)".into()),
        DeadEndOutcome::Clean { depth } => (true, format!("CLEAN (checked depth {depth})")),
        DeadEndOutcome::InitialUnsat => {
            issues.push("over-constraint: assumptions are contradictory at cycle 0".into());
            (false, "DEAD-END (initial UNSAT)".into())
        }
        DeadEndOutcome::DeadEndFound { step, trace } => {
            issues.push(format!(
                "over-constraint: assumptions reach a dead-end state at cycle {step}"
            ));
            if failure_trace_table.is_none() {
                failure_backtrace = Some(compute_cex_backtrace(m, None, &trace));
                failure_trace_table = Some(format_agent_trace_table(m, None, &trace, 12));
            }
            (false, format!("DEAD-END (@ t={step})"))
        }
        DeadEndOutcome::Unknown { step } => {
            issues.push(format!("dead-end check inconclusive at cycle {step}"));
            (false, format!("UNKNOWN (@ t={step})"))
        }
    };

    // Gate 3: Vacuity Detection.
    let mut vacuous_properties = Vec::new();
    if assert_count > 0 && deadend_ok {
        let vacs = check_vacuity(
            m,
            &VacuityConfig {
                depth: cfg.depth.min(12),
                solver: cfg.solver.clone(),
                use_pdr: true,
            },
        )?;
        for v in vacs {
            if matches!(
                v.status,
                VacuityStatus::Vacuous | VacuityStatus::BoundedVacuous { .. }
            ) {
                vacuous_properties.push(v.property.clone());
                issues.push(format!(
                    "vacuous assertion `{}` (antecedent/guard never triggers)",
                    v.property
                ));
            }
        }
    }
    let non_vacuous = vacuous_properties.is_empty();

    // Gate 4: Formal Mutation Testing (only meaningful when proof passes & no dead-end).
    let (mutation_total, mutation_killed, mutation_score_pct, surviving_mutants, mutation_ok) =
        if proof_ok && deadend_ok && cfg.max_mutants > 0 {
            let mrep = run_mutation_analysis(
                m,
                &MutationConfig {
                    depth: cfg.depth.min(10),
                    max_mutants: cfg.max_mutants,
                    solver: cfg.solver.clone(),
                },
            )?;
            let survivors: Vec<String> = mrep
                .mutants
                .iter()
                .filter(|x| x.status == MutantStatus::Survived)
                .map(|x| x.description.clone())
                .collect();
            let ok = mrep.total == 0 || mrep.score_percent >= cfg.min_mutation_score;
            if !ok {
                issues.push(format!(
                    "weak specification: mutation score {:.1}% is below minimum {:.1}% ({} surviving mutants)",
                    mrep.score_percent,
                    cfg.min_mutation_score,
                    survivors.len()
                ));
            }
            (mrep.total, mrep.killed, mrep.score_percent, survivors, ok)
        } else {
            (0, 0, 0.0, Vec::new(), !proof_ok)
        };

    let trustworthy =
        assert_count > 0 && proof_ok && deadend_ok && non_vacuous && mutation_ok;

    Ok(TrustAuditReport {
        module_name: m.name.clone(),
        trustworthy,
        assert_count,
        assume_count,
        proof_status,
        deadend_status,
        vacuous_properties,
        mutation_total,
        mutation_killed,
        mutation_score_pct,
        surviving_mutants,
        issues,
        failure_backtrace,
        failure_trace_table,
    })
}
