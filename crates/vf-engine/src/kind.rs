use anyhow::Result;
use std::sync::Arc;
use vf_ir::{Module, PropertyKind};
use vf_smt::smt2;
use vf_smt::solver::{CancelToken, SatResult, SmtSolver, TraceFrame};

#[derive(Debug, Clone)]
pub struct ProveConfig {
    pub max_k: usize,
    pub solver: String,
}

impl Default for ProveConfig {
    fn default() -> Self {
        Self {
            max_k: 20,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProveOutcome {
    /// Unbounded safety proof established by k-induction!
    Proved {
        k: usize,
    },
    /// Counterexample found in the base case.
    Fail {
        property: String,
        step: usize,
        trace: Vec<TraceFrame>,
    },
    AssumptionsUnsat {
        step: usize,
    },
    /// Neither proved nor falsified within `max_k`.
    Unknown {
        max_k: usize,
    },
}

/// k-Induction unbounded safety prover (base case + inductive step, analogous to `sby mode prove`).
pub fn run_kinduction(m: &Module, cfg: &ProveConfig) -> Result<ProveOutcome> {
    run_kinduction_cancellable(m, cfg, None)
}

/// Cancellable k-induction runner used by the parallel portfolio so losing solver subprocesses
/// terminate immediately when another engine finishes first.
pub fn run_kinduction_cancellable(
    m: &Module,
    cfg: &ProveConfig,
    cancel: Option<Arc<CancelToken>>,
) -> Result<ProveOutcome> {
    match run_kinduction_inner(m, cfg, cancel.clone()) {
        Ok(out) => Ok(out),
        Err(_) if cancel.as_ref().is_some_and(|c| c.is_cancelled()) => {
            Ok(ProveOutcome::Unknown { max_k: 0 })
        }
        Err(e) => Err(e),
    }
}

fn run_kinduction_inner(
    m: &Module,
    cfg: &ProveConfig,
    cancel: Option<Arc<CancelToken>>,
) -> Result<ProveOutcome> {
    let assert_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Assert)
        .map(|(i, _)| i)
        .collect();

    if assert_indices.is_empty() {
        return Ok(ProveOutcome::Proved { k: 0 });
    }
    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let mut base = SmtSolver::spawn_with_cancel(&cfg.solver, cancel.clone())?;
    base.send(&smt2::header(m))?;

    let mut ind = SmtSolver::spawn_with_cancel(&cfg.solver, cancel.clone())?;
    ind.send(&smt2::header(m))?;

    let init_sig = m.find_signal("$initstate");

    for k in 0..cfg.max_k {
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Ok(ProveOutcome::Unknown { max_k: k });
        }
        // --- 1. Base case at step k ---
        base.send(&smt2::encode_step(m, k, k == 0, k.checked_sub(1)))?;
        base.send(&smt2::encode_assumes(m, k))?;

        base.push()?;
        let mut base_fails = Vec::with_capacity(assert_indices.len());
        for &idx in &assert_indices {
            let (defs, fail) = smt2::encode_assert_failure(m, idx, k);
            base.send(&defs)?;
            base_fails.push(fail);
        }
        let base_disj = disjunction(&base_fails);
        base.send(&format!("(assert {base_disj})\n"))?;

        match base.check_sat()? {
            SatResult::Sat => {
                let trace = base.extract_trace(m, k + 1)?;
                base.pop()?;
                let failed = find_failing_assertion(&mut base, m, &assert_indices, k)?;
                return Ok(ProveOutcome::Fail {
                    property: failed,
                    step: k,
                    trace,
                });
            }
            SatResult::Unknown => {
                base.pop()?;
                return Ok(ProveOutcome::Unknown { max_k: k });
            }
            SatResult::Unsat => {
                base.pop()?;
                if k == 0 && base.check_sat()? == SatResult::Unsat {
                    return Ok(ProveOutcome::AssumptionsUnsat { step: 0 });
                }
                // Assertions hold at step k in base case; we can assume them to speed up later steps.
                base.send(&smt2::encode_asserts_hold(m, k))?;
            }
        }

        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Ok(ProveOutcome::Unknown { max_k: k });
        }

        // --- 2. Inductive step at step k ---
        // Unconstrained starting state at step 0 (no `apply_init`), except if `$initstate == 1`
        // at step 0, then actual initial-state FF constraints apply.
        ind.send(&smt2::encode_step(m, k, false, k.checked_sub(1)))?;
        if k == 0 {
            if let Some(isig) = init_sig {
                let isig_name = smt2::sig_name(isig, 0);
                for ff in &m.ffs {
                    if ff.signal != isig {
                        if let Some(init) = &ff.init {
                            ind.send(&format!(
                                "(assert (=> (= {isig_name} #b1) (= {} {})))\n",
                                smt2::sig_name(ff.signal, 0),
                                smt2::smt_const(init)
                            ))?;
                        }
                    }
                }
            }
        }
        ind.send(&smt2::encode_assumes(m, k))?;

        // Check if any assertion can fail at step k given that assertions held at 0..k.
        ind.push()?;
        let mut ind_fails = Vec::with_capacity(assert_indices.len());
        for &idx in &assert_indices {
            let (defs, fail) = smt2::encode_assert_failure(m, idx, k);
            ind.send(&defs)?;
            ind_fails.push(fail);
        }
        let ind_disj = disjunction(&ind_fails);
        ind.send(&format!("(assert {ind_disj})\n"))?;

        let ind_res = ind.check_sat()?;
        ind.pop()?;

        match ind_res {
            SatResult::Unsat => {
                // Both base case (0..=k) and inductive step (k) succeeded -> UNBOUNDED PROOF!
                return Ok(ProveOutcome::Proved { k });
            }
            SatResult::Sat => {
                // Inductive hypothesis: assert that all assertions hold at step k before moving to k+1.
                ind.send(&smt2::encode_asserts_hold(m, k))?;
            }
            SatResult::Unknown => {
                return Ok(ProveOutcome::Unknown { max_k: k });
            }
        }
    }

    Ok(ProveOutcome::Unknown { max_k: cfg.max_k })
}

fn disjunction(clauses: &[String]) -> String {
    if clauses.len() == 1 {
        clauses[0].clone()
    } else {
        format!("(or {})", clauses.join(" "))
    }
}

fn find_failing_assertion(
    solver: &mut SmtSolver,
    m: &Module,
    indices: &[usize],
    step: usize,
) -> Result<String> {
    for &idx in indices {
        solver.push()?;
        let (defs, fail) = smt2::encode_assert_failure(m, idx, step);
        solver.send(&defs)?;
        solver.send(&format!("(assert {fail})\n"))?;
        let sat = solver.check_sat()?;
        solver.pop()?;
        if sat == SatResult::Sat {
            return Ok(m.properties[idx].name.clone());
        }
    }
    Ok(m.properties[indices[0]].name.clone())
}
