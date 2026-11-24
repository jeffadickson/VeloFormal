use anyhow::Result;
use std::collections::HashSet;
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{ConstBits, ExprId, ExprKind, Module, Property, PropertyKind, SignalId, SignalKind};
use vf_slang::{compile_to_ir_with_options, CompileOptions, InstrumentOptions};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct AutoCheckConfig {
    pub depth: usize,
    pub solver: String,
    pub check_overflow: bool,
    /// Check whether any initialized user FF is provably stuck constant forever (`reg == init`).
    pub check_const_regs: bool,
    /// Optional FSM state register names to check for deadlock, livelock, and unreachable states.
    pub fsm_signals: Vec<String>,
    pub use_pdr: bool,
}

impl Default for AutoCheckConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            solver: "z3".to_string(),
            check_overflow: false,
            check_const_regs: false,
            fsm_signals: Vec::new(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CheckStatus {
    Safe,
    Bug { step: usize, trace: Vec<TraceFrame> },
    Warning(String),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct AutoCheckFinding {
    pub category: String,
    pub name: String,
    pub status: CheckStatus,
}

/// Questa AutoCheck equivalent: push-button formal RTL bug hunting requiring ZERO SVA or testbench.
/// Automatically checks:
/// - Out-of-bounds memory/array indexing
/// - Divide/modulo by zero
/// - Out-of-range shift amounts (`rhs >= width(lhs)`)
/// - `unique` / `priority` `case` & `if` overlap and completeness
/// - Optional unsigned arithmetic overflow/underflow
/// - FSM deadlock, FSM livelock, and FSM unreachable declared states
/// - Structural lint (undriven internal wires, clock-used-as-data)
/// - Optional constant-stuck register detection
pub fn run_autocheck(
    compile: &CompileOptions,
    cfg: &AutoCheckConfig,
) -> Result<(Module, Vec<AutoCheckFinding>)> {
    let inst = InstrumentOptions {
        branch_covers: false,
        unique_priority_checks: true,
        oob_memory_checks: true,
        div_zero_checks: true,
        overflow_checks: cfg.check_overflow,
        shift_overflow_checks: true,
    };
    let mut m = compile_to_ir_with_options(compile, inst)?;

    let mut findings = Vec::new();

    // 0. Structural clock-used-as-data check (MUST run BEFORE clk2fflogic!).
    let clock_sigs: HashSet<SignalId> = m
        .signals
        .iter()
        .filter(|s| s.is_clock)
        .map(|s| s.id)
        .chain(m.ffs.iter().filter_map(|f| f.clock.map(|c| c.signal)))
        .collect();
    if !clock_sigs.is_empty() {
        let mut data_used_clocks: HashSet<SignalId> = HashSet::new();
        for ca in &m.comb {
            if !clock_sigs.contains(&ca.signal) {
                collect_used_clocks(&m, ca.value, &clock_sigs, &mut data_used_clocks);
            }
        }
        for ff in &m.ffs {
            collect_used_clocks(&m, ff.next, &clock_sigs, &mut data_used_clocks);
        }
        for csid in data_used_clocks {
            let cname = &m.signal(csid).name;
            if !cname.starts_with('$') {
                findings.push(AutoCheckFinding {
                    category: "clock_as_data".into(),
                    name: cname.clone(),
                    status: CheckStatus::Warning(format!(
                        "clock signal '{cname}' is referenced inside data/combinational logic"
                    )),
                });
            }
        }
    }

    if vf_ir::passes::has_multiple_clocks(&m) {
        vf_ir::passes::clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        vf_ir::passes::async2sync(&mut m);
    }

    // 1. Structural lint: detect undriven internal wires/outputs.
    let driven: HashSet<_> = m
        .comb
        .iter()
        .map(|c| c.signal)
        .chain(m.ffs.iter().map(|f| f.signal))
        .collect();
    for s in &m.signals {
        if matches!(s.kind, SignalKind::Output | SignalKind::Wire)
            && !s.name.starts_with('$')
            && !driven.contains(&s.id)
        {
            findings.push(AutoCheckFinding {
                category: "undriven".into(),
                name: s.name.clone(),
                status: CheckStatus::Warning(format!("signal '{}' has no driver", s.name)),
            });
        }
    }

    // 2. Check each auto-generated assertion independently (with COI pruning!).
    let auto_asserts: Vec<Property> = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assert && p.name.starts_with("autocheck_"))
        .cloned()
        .collect();

    for prop in auto_asserts {
        let category = if prop.name.contains("oob_mem") {
            "oob_memory"
        } else if prop.name.contains("div_by_zero") {
            "div_by_zero"
        } else if prop.name.contains("shift_overflow") {
            "shift_overflow"
        } else if prop.name.contains("overflow") {
            "overflow"
        } else {
            "unique_priority"
        }
        .to_string();

        let mut sub = m.clone();
        sub.properties.retain(|p| p.kind == PropertyKind::Assume);
        sub.properties.push(prop.clone());
        vf_ir::passes::coi_prune(&mut sub);

        let status = verify_single_assert(&sub, cfg)?;
        findings.push(AutoCheckFinding {
            category,
            name: prop.name,
            status,
        });
    }

    // 3. Optional constant-stuck register check: prove if any initialized user FF can never change.
    if cfg.check_const_regs {
        for ff in &m.ffs {
            let s = m.signal(ff.signal);
            if s.name.starts_with('$') || s.is_clock {
                continue;
            }
            if let Some(init) = &ff.init {
                let mut sub = m.clone();
                sub.properties.retain(|p| p.kind == PropertyKind::Assume);
                let se = sub.sig_expr(ff.signal);
                let ce = sub.c_bits(init.clone());
                let eq = sub.eq(se, ce, false);
                let one = sub.c_bool(true);
                sub.properties.push(Property {
                    name: format!("const_reg_{}", s.name),
                    kind: PropertyKind::Assert,
                    cond: eq,
                    en: one,
                    source: None,
                });
                vf_ir::passes::coi_prune(&mut sub);
                if let Ok(ProveOutcome::Proved { .. }) = run_kinduction(
                    &sub,
                    &ProveConfig {
                        max_k: 2,
                        solver: cfg.solver.clone(),
                    },
                ) {
                    findings.push(AutoCheckFinding {
                        category: "const_reg".into(),
                        name: s.name.clone(),
                        status: CheckStatus::Warning(format!(
                            "register '{}' is provably stuck constant at {}'b{}",
                            s.name, init.width, init.bits
                        )),
                    });
                }
            }
        }
    }

    // 4. FSM analysis (deadlock, livelock, and unreachable declared states) for each `--fsm`.
    for fsm_name in &cfg.fsm_signals {
        if let Some(sig) = m.find_signal(fsm_name) {
            if let Some(ff) = m.ffs.iter().find(|f| f.signal == sig).cloned() {
                let dl_status = check_fsm_deadlock(&m, sig, ff.next, cfg)?;
                findings.push(AutoCheckFinding {
                    category: "fsm_deadlock".into(),
                    name: fsm_name.clone(),
                    status: dl_status,
                });

                let declared_states = extract_fsm_states(&m, sig, ff.next);
                if !declared_states.is_empty() {
                    let unreach = check_fsm_unreachable(&m, sig, fsm_name, &declared_states, cfg)?;
                    findings.extend(unreach);

                    let ll_status =
                        check_fsm_livelock(&m, sig, ff.next, ff.init.as_ref(), &declared_states, cfg)?;
                    findings.push(AutoCheckFinding {
                        category: "fsm_livelock".into(),
                        name: fsm_name.clone(),
                        status: ll_status,
                    });
                }
            }
        }
    }

    Ok((m, findings))
}

fn collect_used_clocks(
    m: &Module,
    root: ExprId,
    clocks: &HashSet<SignalId>,
    found: &mut HashSet<SignalId>,
) {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(eid) = stack.pop() {
        if !visited.insert(eid) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Signal(sid) => {
                if clocks.contains(sid) {
                    found.insert(*sid);
                }
            }
            ExprKind::Const(_) => {}
            ExprKind::Not(a)
            | ExprKind::Neg(a)
            | ExprKind::RedAnd(a)
            | ExprKind::RedOr(a)
            | ExprKind::RedXor(a)
            | ExprKind::Slice { expr: a, .. }
            | ExprKind::Zext { expr: a, .. }
            | ExprKind::Sext { expr: a, .. }
            | ExprKind::MemRead { addr: a, .. } => stack.push(*a),
            ExprKind::Add(a, b)
            | ExprKind::Sub(a, b)
            | ExprKind::Mul(a, b)
            | ExprKind::UDiv(a, b)
            | ExprKind::SDiv(a, b)
            | ExprKind::UMod(a, b)
            | ExprKind::SMod(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b)
            | ExprKind::Xor(a, b)
            | ExprKind::Shl(a, b)
            | ExprKind::Lshr(a, b)
            | ExprKind::Ashr(a, b)
            | ExprKind::Eq(a, b)
            | ExprKind::Ne(a, b)
            | ExprKind::Ult(a, b)
            | ExprKind::Ule(a, b)
            | ExprKind::Slt(a, b)
            | ExprKind::Sle(a, b) => {
                stack.push(*a);
                stack.push(*b);
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                stack.push(*cond);
                stack.push(*then_val);
                stack.push(*else_val);
            }
            ExprKind::Concat(parts) => stack.extend(parts.iter().copied()),
        }
    }
}

/// Extract declared FSM state encodings referenced by equality checks against `sig` or
/// constant assignments inside `next_expr`.
fn extract_fsm_states(m: &Module, sig: SignalId, next_expr: ExprId) -> Vec<ConstBits> {
    let w = m.signal(sig).width;
    let mut vals: HashSet<ConstBits> = HashSet::new();

    // 1. Constants compared against `sig` anywhere in the module's expressions (`fsm == STATE`).
    for node in &m.exprs {
        if let ExprKind::Eq(a, b) = &node.kind {
            match (&m.expr(*a).kind, &m.expr(*b).kind) {
                (ExprKind::Signal(s), ExprKind::Const(c))
                | (ExprKind::Const(c), ExprKind::Signal(s))
                    if *s == sig && c.width == w =>
                {
                    vals.insert(c.clone());
                }
                _ => {}
            }
        }
    }

    // 2. Constants assigned to `sig` as leaves of `Mux` trees in `next_expr`.
    let mut stack = vec![next_expr];
    let mut seen = HashSet::new();
    while let Some(eid) = stack.pop() {
        if !seen.insert(eid) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Const(c) if c.width == w => {
                vals.insert(c.clone());
            }
            ExprKind::Mux {
                then_val, else_val, ..
            } => {
                stack.push(*then_val);
                stack.push(*else_val);
            }
            _ => {}
        }
    }

    if let Some(init) = &m.signal(sig).init {
        if init.width == w {
            vals.insert(init.clone());
        }
    }

    let mut sorted: Vec<ConstBits> = vals.into_iter().collect();
    sorted.sort_by(|a, b| a.bits.cmp(&b.bits));
    sorted.truncate(32);
    sorted
}

/// Check every declared FSM state encoding for reachability; prove unbounded unreachability for dead states.
fn check_fsm_unreachable(
    m: &Module,
    sig: SignalId,
    fsm_name: &str,
    states: &[ConstBits],
    cfg: &AutoCheckConfig,
) -> Result<Vec<AutoCheckFinding>> {
    let mut sub = m.clone();
    sub.properties.retain(|p| p.kind == PropertyKind::Assume);
    let sig_e = sub.sig_expr(sig);
    let one = sub.c_bool(true);
    for st in states {
        let ce = sub.c_bits(st.clone());
        let eq = sub.eq(sig_e, ce, false);
        sub.properties.push(Property {
            name: format!("fsm_state_{}", st.bits),
            kind: PropertyKind::Cover,
            cond: eq,
            en: one,
            source: None,
        });
    }

    let cover_res = run_cover(
        &sub,
        &CoverConfig {
            depth: cfg.depth,
            solver: cfg.solver.clone(),
            append: 0,
        },
    )?;

    let mut out = Vec::new();
    let mut any_unreach = false;
    for (st, res) in states.iter().zip(cover_res.iter()) {
        if res.reached.is_none() {
            // Confirm unbounded unreachability via k-induction/PDR (`assert (sig != st)`).
            let mut prove_mod = m.clone();
            prove_mod
                .properties
                .retain(|p| p.kind == PropertyKind::Assume);
            let se = prove_mod.sig_expr(sig);
            let ce = prove_mod.c_bits(st.clone());
            let neq = prove_mod.intern(1, ExprKind::Ne(se, ce));
            let one_p = prove_mod.c_bool(true);
            prove_mod.properties.push(Property {
                name: format!("fsm_unreach_{}", st.bits),
                kind: PropertyKind::Assert,
                cond: neq,
                en: one_p,
                source: None,
            });
            vf_ir::passes::coi_prune(&mut prove_mod);
            if matches!(verify_single_assert(&prove_mod, cfg)?, CheckStatus::Safe) {
                any_unreach = true;
                let label = st
                    .to_u64()
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| format!("{}'b{}", st.width, st.bits));
                out.push(AutoCheckFinding {
                    category: "fsm_unreachable".into(),
                    name: format!("{fsm_name}=={label}"),
                    status: CheckStatus::Warning(format!(
                        "declared FSM state '{fsm_name} == {label}' is formally unreachable"
                    )),
                });
            }
        }
    }

    if !any_unreach {
        out.push(AutoCheckFinding {
            category: "fsm_unreachable".into(),
            name: fsm_name.to_string(),
            status: CheckStatus::Safe,
        });
    }

    Ok(out)
}

/// Detect multi-state FSM livelock: a reachable non-initial state `s != init` where the FSM can
/// still transition (`next != cur`, so not a single-state deadlock) yet CANNOT return to `init`
/// within `depth` cycles without asserting reset.
fn check_fsm_livelock(
    m: &Module,
    sig: SignalId,
    next_expr: ExprId,
    init_opt: Option<&ConstBits>,
    states: &[ConstBits],
    cfg: &AutoCheckConfig,
) -> Result<CheckStatus> {
    use vf_smt::smt2;
    use vf_smt::solver::{SatResult, SmtSolver};

    let w = m.signal(sig).width;
    let init_val = init_opt
        .cloned()
        .unwrap_or_else(|| ConstBits::zero(w));

    // Detect optional reset input so livelock tests escape WITHOUT relying on hardware reset.
    let rst_info: Option<(SignalId, bool)> = ["rst", "rst_n", "reset", "reset_n", "presetn"]
        .iter()
        .find_map(|name| {
            m.find_signal(name).and_then(|sid| {
                if m.signal(sid).kind == SignalKind::Input && m.signal(sid).width == 1 {
                    let low = name.ends_with("_n") || name.ends_with("resetn");
                    Some((sid, low))
                } else {
                    None
                }
            })
        });

    let mut dm = m.clone();
    let cur_e = dm.sig_expr(sig);
    let moving_e = dm.intern(1, ExprKind::Ne(next_expr, cur_e));
    let moving_sig = dm.add_signal("$fsm_moving", 1, false, SignalKind::Wire);
    dm.comb.push(vf_ir::CombAssign {
        signal: moving_sig,
        value: moving_e,
    });
    let _ = dm.topo_sort_comb();

    // Find shortest trace reaching each non-init declared state (with reset inactive at arrival).
    let escape_depth = cfg.depth.clamp(4, 12);
    for st in states {
        if st == &init_val {
            continue;
        }
        // Check if state `st` is reachable.
        let mut cov_m = dm.clone();
        cov_m.properties.retain(|p| p.kind == PropertyKind::Assume);
        let se = cov_m.sig_expr(sig);
        let ce = cov_m.c_bits(st.clone());
        let eq = cov_m.eq(se, ce, false);
        let en = if let Some((rsid, active_low)) = rst_info {
            let re = cov_m.sig_expr(rsid);
            if active_low {
                re
            } else {
                cov_m.logic_not(re)
            }
        } else {
            cov_m.c_bool(true)
        };
        cov_m.properties.push(Property {
            name: "reach_st".into(),
            kind: PropertyKind::Cover,
            cond: eq,
            en,
            source: None,
        });
        let cov = run_cover(
            &cov_m,
            &CoverConfig {
                depth: cfg.depth,
                solver: cfg.solver.clone(),
                append: 0,
            },
        )?;
        let Some((reach_step, trace)) = cov.into_iter().next().and_then(|r| r.reached) else {
            continue;
        };

        // First verify it is NOT a single-state deadlock (i.e. `moving == 1` is possible from `trace[reach_step]`).
        let mut mv = SmtSolver::spawn(&cfg.solver)?;
        mv.send(&smt2::header(&dm))?;
        mv.send(&smt2::encode_step(&dm, 0, false, None))?;
        mv.send(&smt2::encode_assumes(&dm, 0))?;
        for ff in &m.ffs {
            let fname = &m.signal(ff.signal).name;
            if let Some((_, v)) = trace[reach_step].values.iter().find(|(n, _)| n == fname) {
                mv.send(&format!(
                    "(assert (= {} {}))\n",
                    smt2::sig_name(ff.signal, 0),
                    smt2::smt_const(v)
                ))?;
            }
        }
        if let Some((rsid, active_low)) = rst_info {
            let inactive = if active_low { "#b1" } else { "#b0" };
            mv.send(&format!(
                "(assert (= {} {inactive}))\n",
                smt2::sig_name(rsid, 0)
            ))?;
        }
        mv.send(&format!(
            "(assert (= {} #b1))\n",
            smt2::sig_name(moving_sig, 0)
        ))?;
        if mv.check_sat()? != SatResult::Sat {
            // Single-state deadlock (already caught by `fsm_deadlock`).
            continue;
        }

        // Now test if ANY input sequence (with reset inactive) can return `sig` to `init_val` within `escape_depth` steps.
        let mut esc = SmtSolver::spawn(&cfg.solver)?;
        esc.send(&smt2::header(&dm))?;
        esc.send(&smt2::encode_step(&dm, 0, false, None))?;
        esc.send(&smt2::encode_assumes(&dm, 0))?;
        for ff in &m.ffs {
            let fname = &m.signal(ff.signal).name;
            if let Some((_, v)) = trace[reach_step].values.iter().find(|(n, _)| n == fname) {
                esc.send(&format!(
                    "(assert (= {} {}))\n",
                    smt2::sig_name(ff.signal, 0),
                    smt2::smt_const(v)
                ))?;
            }
        }
        if let Some((rsid, active_low)) = rst_info {
            let inactive = if active_low { "#b1" } else { "#b0" };
            esc.send(&format!(
                "(assert (= {} {inactive}))\n",
                smt2::sig_name(rsid, 0)
            ))?;
        }

        let init_smt = smt2::smt_const(&init_val);
        let mut can_return = false;
        for step in 1..=escape_depth {
            esc.send(&smt2::encode_step(&dm, step, false, Some(step - 1)))?;
            esc.send(&smt2::encode_assumes(&dm, step))?;
            if let Some((rsid, active_low)) = rst_info {
                let inactive = if active_low { "#b1" } else { "#b0" };
                esc.send(&format!(
                    "(assert (= {} {inactive}))\n",
                    smt2::sig_name(rsid, step)
                ))?;
            }
            esc.push()?;
            esc.send(&format!(
                "(assert (= {} {init_smt}))\n",
                smt2::sig_name(sig, step)
            ))?;
            if esc.check_sat()? == SatResult::Sat {
                can_return = true;
                esc.pop()?;
                break;
            }
            esc.pop()?;
        }

        if !can_return {
            return Ok(CheckStatus::Bug {
                step: reach_step,
                trace,
            });
        }
    }

    Ok(CheckStatus::Safe)
}

fn verify_single_assert(m: &Module, cfg: &AutoCheckConfig) -> Result<CheckStatus> {
    let k = run_kinduction(
        m,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )?;
    match k {
        ProveOutcome::Proved { .. } => Ok(CheckStatus::Safe),
        ProveOutcome::Fail { step, trace, .. } => Ok(CheckStatus::Bug { step, trace }),
        ProveOutcome::AssumptionsUnsat { .. } => Ok(CheckStatus::Safe),
        ProveOutcome::Unknown { .. } => {
            if cfg.use_pdr {
                if let Ok(p) = run_pdr(
                    m,
                    &PdrConfig {
                        smt_solver: cfg.solver.clone(),
                        ..Default::default()
                    },
                ) {
                    return match p {
                        ProveOutcome::Proved { .. } => Ok(CheckStatus::Safe),
                        ProveOutcome::Fail { step, trace, .. } => {
                            Ok(CheckStatus::Bug { step, trace })
                        }
                        _ => Ok(CheckStatus::Unknown),
                    };
                }
            }
            Ok(CheckStatus::Unknown)
        }
    }
}

/// Detect whether an FSM register `sig` has a reachable deadlock state where `next == cur`
/// regardless of primary inputs (i.e. even when trying to escape, `next(s, in) == s` for all `in`).
fn check_fsm_deadlock(
    m: &Module,
    sig: vf_ir::SignalId,
    next_expr: vf_ir::ExprId,
    cfg: &AutoCheckConfig,
) -> Result<CheckStatus> {
    use vf_smt::smt2;
    use vf_smt::solver::{SatResult, SmtSolver};

    let inputs: Vec<_> = m
        .signals
        .iter()
        .filter(|s| s.kind == SignalKind::Input && !s.is_clock)
        .collect();

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send("(set-logic ALL)\n(set-option :produce-models true)\n")?;

    let mut dm = m.clone();
    let cur_e = dm.sig_expr(sig);
    let stuck_e = dm.intern(1, ExprKind::Eq(next_expr, cur_e));
    let stuck_sig = dm.add_signal("$fsm_stuck", 1, false, SignalKind::Wire);
    dm.comb.push(vf_ir::CombAssign {
        signal: stuck_sig,
        value: stuck_e,
    });
    let _ = dm.topo_sort_comb();

    for t in 0..cfg.depth {
        solver.send(&smt2::encode_step(&dm, t, t == 0, t.checked_sub(1)))?;
        solver.send(&smt2::encode_assumes(&dm, t))?;

        solver.push()?;
        solver.send(&format!(
            "(assert (= {} #b1))\n",
            smt2::sig_name(stuck_sig, t)
        ))?;
        let mut blocked_states: Vec<String> = Vec::new();
        loop {
            match solver.check_sat()? {
                SatResult::Sat => {
                    let trace = solver.extract_trace(&dm, t + 1)?;
                    let cur_val = trace[t]
                        .values
                        .iter()
                        .find(|(n, _)| n == &m.signal(sig).name)
                        .map(|(_, v)| v.clone())
                        .unwrap();

                    let mut escape = SmtSolver::spawn(&cfg.solver)?;
                    escape.send(&smt2::header(&dm))?;
                    escape.send(&smt2::encode_step(&dm, 0, false, None))?;
                    escape.send(&smt2::encode_assumes(&dm, 0))?;
                    for ff in &m.ffs {
                        let fname = &m.signal(ff.signal).name;
                        if let Some((_, v)) = trace[t].values.iter().find(|(n, _)| n == fname) {
                            escape.send(&format!(
                                "(assert (= {} {}))\n",
                                smt2::sig_name(ff.signal, 0),
                                smt2::smt_const(v)
                            ))?;
                        }
                    }
                    escape.send(&format!(
                        "(assert (= {} #b0))\n",
                        smt2::sig_name(stuck_sig, 0)
                    ))?;
                    let can_escape = escape.check_sat()?;
                    if can_escape == SatResult::Unsat {
                        solver.pop()?;
                        return Ok(CheckStatus::Bug { step: t, trace });
                    }
                    let val_smt = smt2::smt_const(&cur_val);
                    if blocked_states.len() > 32 || inputs.is_empty() {
                        break;
                    }
                    blocked_states.push(val_smt.clone());
                    solver.send(&format!(
                        "(assert (not (= {} {val_smt})))\n",
                        smt2::sig_name(sig, t)
                    ))?;
                }
                _ => break,
            }
        }
        solver.pop()?;
    }

    Ok(CheckStatus::Safe)
}
