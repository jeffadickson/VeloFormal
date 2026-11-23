use anyhow::Result;
use std::collections::HashSet;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{ExprKind, Module, Property, PropertyKind, SignalKind};
use vf_slang::{compile_to_ir_with_options, CompileOptions, InstrumentOptions};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct AutoCheckConfig {
    pub depth: usize,
    pub solver: String,
    pub check_overflow: bool,
    /// Optional FSM state register names to check for deadlock states.
    pub fsm_signals: Vec<String>,
    pub use_pdr: bool,
}

impl Default for AutoCheckConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            solver: "z3".to_string(),
            check_overflow: false,
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
/// - `unique` / `priority` `case` & `if` overlap and completeness
/// - Optional unsigned arithmetic overflow/underflow
/// - FSM deadlock detection (reachable state trapped forever regardless of inputs)
/// - Structural lint (undriven internal wires)
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
    };
    let mut m = compile_to_ir_with_options(compile, inst)?;
    if vf_ir::passes::has_multiple_clocks(&m) {
        vf_ir::passes::clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        vf_ir::passes::async2sync(&mut m);
    }

    let mut findings = Vec::new();

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

    // 3. FSM deadlock check: for each specified FSM register, check if a reachable state
    //    can get stuck where `next == cur` for a full cycle (or assert `next != cur` fails
    //    under existential? Better: check if there exists a reachable state S where for ALL
    //    inputs in the next cycle, `next(S) == S`, OR detect unreachable/trapped deadlocks
    //    using a 2-copy miter or direct SMT check!).
    for fsm_name in &cfg.fsm_signals {
        if let Some(sig) = m.find_signal(fsm_name) {
            if let Some(ff) = m.ffs.iter().find(|f| f.signal == sig).cloned() {
                let status = check_fsm_deadlock(&m, sig, ff.next, cfg)?;
                findings.push(AutoCheckFinding {
                    category: "fsm_deadlock".into(),
                    name: fsm_name.clone(),
                    status,
                });
            }
        }
    }

    Ok((m, findings))
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

    // Unroll up to `cfg.depth` to reach state `s_t`; at step `t`, check if `s_t` is deadlocked
    // by testing whether `next(s_t) == s_t` holds for ALL input valuations (quantified check
    // or checking if the state's next-expr simplifies to `Signal(sig)` under the reached state).
    // Using SMT-LIB2 `forall` over non-clock primary inputs at step `t` is exact in Z3!
    let inputs: Vec<_> = m
        .signals
        .iter()
        .filter(|s| s.kind == SignalKind::Input && !s.is_clock)
        .collect();

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    // Use BV (allows quantifiers in Z3 for exact deadlock test!).
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

        // Check if current state `s_t` is stuck (`stuck == 1`), then verify via a second
        // solver query with `s_t` fixed whether ANY input can escape (`stuck == 0`).
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

                    // Test if ANY input assignment can leave `cur_val`!
                    let mut escape = SmtSolver::spawn(&cfg.solver)?;
                    escape.send(&smt2::header(&dm))?;
                    escape.send(&smt2::encode_step(&dm, 0, false, None))?;
                    escape.send(&smt2::encode_assumes(&dm, 0))?;
                    // Fix all FF states to their values from `trace[t]`.
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
                    // Ask: can `stuck == 0` (i.e. can the FSM leave this state)?
                    escape.send(&format!(
                        "(assert (= {} #b0))\n",
                        smt2::sig_name(stuck_sig, 0)
                    ))?;
                    let can_escape = escape.check_sat()?;
                    if can_escape == SatResult::Unsat {
                        solver.pop()?;
                        return Ok(CheckStatus::Bug { step: t, trace });
                    }
                    // Not deadlocked — block this state value and check if another reachable state is deadlocked.
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
