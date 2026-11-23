use anyhow::{bail, Result};
use vf_ir::{Module, PropertyKind};
use vf_smt::smt2;
use vf_smt::solver::{SatResult, SmtSolver, TraceFrame};

#[derive(Debug, Clone)]
pub struct BmcConfig {
    pub depth: usize,
    pub solver: String,
    /// Skip checking assertions during cycles `0..skip` (SymbiYosys `skip N`).
    pub skip: usize,
    /// Append N extra cycles after a failing assertion in the counterexample trace (SymbiYosys `append N`).
    pub append: usize,
}

impl Default for BmcConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
            skip: 0,
            append: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BmcOutcome {
    Pass {
        depth: usize,
    },
    Fail {
        property: String,
        step: usize,
        trace: Vec<TraceFrame>,
    },
    AssumptionsUnsat {
        step: usize,
    },
    Unknown {
        step: usize,
    },
}

pub fn run_bmc(m: &Module, cfg: &BmcConfig) -> Result<BmcOutcome> {
    let assert_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Assert)
        .map(|(i, _)| i)
        .collect();

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send(&smt2::header(m))?;

    for t in 0..cfg.depth {
        solver.send(&smt2::encode_step(m, t, t == 0, t.checked_sub(1)))?;
        solver.send(&smt2::encode_assumes(m, t))?;

        if assert_indices.is_empty() || t < cfg.skip {
            continue;
        }

        solver.push()?;
        let mut clauses = Vec::with_capacity(assert_indices.len());
        for &idx in &assert_indices {
            let (defs, fail) = smt2::encode_assert_failure(m, idx, t);
            solver.send(&defs)?;
            clauses.push(fail);
        }
        let disj = if clauses.len() == 1 {
            clauses[0].clone()
        } else {
            format!("(or {})", clauses.join(" "))
        };
        solver.send(&format!("(assert {disj})\n"))?;

        match solver.check_sat()? {
            SatResult::Sat => {
                // Optionally append extra cycles after the failure (SymbiYosys `append N`).
                let mut total_steps = t + 1;
                if cfg.append > 0 {
                    for extra in 1..=cfg.append {
                        let nt = t + extra;
                        solver.send(&smt2::encode_step(m, nt, false, Some(nt - 1)))?;
                        solver.send(&smt2::encode_assumes(m, nt))?;
                    }
                    if solver.check_sat()? == SatResult::Sat {
                        total_steps = t + 1 + cfg.append;
                    }
                }
                let trace = solver.extract_trace(m, total_steps)?;
                solver.pop()?;

                let mut failed_name = m.properties[assert_indices[0]].name.clone();
                if assert_indices.len() > 1 {
                    for &idx in &assert_indices {
                        solver.push()?;
                        let (defs, fail) = smt2::encode_assert_failure(m, idx, t);
                        solver.send(&defs)?;
                        solver.send(&format!("(assert {fail})\n"))?;
                        let sat = solver.check_sat()?;
                        solver.pop()?;
                        if sat == SatResult::Sat {
                            failed_name = m.properties[idx].name.clone();
                            break;
                        }
                    }
                }
                return Ok(BmcOutcome::Fail {
                    property: failed_name,
                    step: t,
                    trace,
                });
            }
            SatResult::Unknown => {
                solver.pop()?;
                return Ok(BmcOutcome::Unknown { step: t });
            }
            SatResult::Unsat => {
                solver.pop()?;
                if t == 0 {
                    match solver.check_sat()? {
                        SatResult::Unsat => return Ok(BmcOutcome::AssumptionsUnsat { step: t }),
                        SatResult::Unknown => return Ok(BmcOutcome::Unknown { step: t }),
                        SatResult::Sat => {}
                    }
                }
            }
        }
    }

    if assert_indices.is_empty() && cfg.depth == 0 {
        bail!("depth must be >= 1");
    }

    Ok(BmcOutcome::Pass { depth: cfg.depth })
}
