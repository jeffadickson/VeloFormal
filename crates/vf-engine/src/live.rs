use anyhow::Result;
use vf_ir::{Module, PropertyKind};
use vf_smt::smt2;
use vf_smt::solver::{SatResult, SmtSolver, TraceFrame};

#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub depth: usize,
    pub solver: String,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LiveOutcome {
    /// No fair starving lasso loop found up to `depth`.
    Pass { depth: usize },
    /// Found a fair infinite lasso `0 -> ... -> loop_start -> ... -> loop_end (== loop_start)`
    /// on which the liveness property never holds inside `[loop_start..loop_end)`.
    Fail {
        property: String,
        loop_start: usize,
        loop_end: usize,
        trace: Vec<TraceFrame>,
    },
    Unknown { step: usize },
}

/// SymbiYosys `mode live` liveness & fairness checker (Biere bounded lasso-loop encoding).
pub fn run_live(m: &Module, cfg: &LiveConfig) -> Result<LiveOutcome> {
    let live_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Live)
        .map(|(i, _)| i)
        .collect();
    let fair_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Fair)
        .map(|(i, _)| i)
        .collect();

    if live_indices.is_empty() {
        return Ok(LiveOutcome::Pass { depth: cfg.depth });
    }

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send(&smt2::header(m))?;

    // Step 0
    solver.send(&smt2::encode_step(m, 0, true, None))?;
    solver.send(&smt2::encode_assumes(m, 0))?;

    for k in 1..=cfg.depth {
        solver.send(&smt2::encode_step(m, k, false, Some(k - 1)))?;
        solver.send(&smt2::encode_assumes(m, k))?;

        // Check if any liveness property has a fair lasso loop ending at step `k`
        // and looping back to some `l` in `0..k`.
        for &live_idx in &live_indices {
            for l in 0..k {
                solver.push()?;

                // 1. State equality: state(k) == state(l) for all FFs and memories.
                let mut eq_terms = Vec::new();
                for ff in &m.ffs {
                    // `$initstate` is 1 at 0 and 0 thereafter, so a loop back to 0 only makes
                    // sense if `$initstate` isn't a real state distinction; skip `$initstate`
                    // if l == 0? No: in Yosys, `$initstate` is a real state register, so loops
                    // can only close to states where `$initstate` matches (i.e. l >= 1 when `$initstate` exists).
                    eq_terms.push(format!(
                        "(= {} {})",
                        smt2::sig_name(ff.signal, k),
                        smt2::sig_name(ff.signal, l)
                    ));
                }
                for mem in &m.memories {
                    eq_terms.push(format!(
                        "(= {} {})",
                        smt2::mem_name(mem.id, k),
                        smt2::mem_name(mem.id, l)
                    ));
                }
                if !eq_terms.is_empty() {
                    let loop_cond = if eq_terms.len() == 1 {
                        eq_terms[0].clone()
                    } else {
                        format!("(and {})", eq_terms.join(" "))
                    };
                    solver.send(&format!("(assert {loop_cond})\n"))?;
                }

                // 2. Starvation: for every t in l..k, the live property is NOT satisfied (en && !cond).
                for t in l..k {
                    let (defs, fail) = smt2::encode_assert_failure(m, live_idx, t);
                    solver.send(&defs)?;
                    solver.send(&format!("(assert {fail})\n"))?;
                }

                // 3. Fairness: for every Fair property, it MUST hold at least once in l..k.
                for &fair_idx in &fair_indices {
                    let mut fair_hits = Vec::new();
                    for t in l..k {
                        let (defs, hit) = smt2::encode_cover_hit(m, fair_idx, t);
                        solver.send(&defs)?;
                        fair_hits.push(hit);
                    }
                    let disj = if fair_hits.len() == 1 {
                        fair_hits[0].clone()
                    } else {
                        format!("(or {})", fair_hits.join(" "))
                    };
                    solver.send(&format!("(assert {disj})\n"))?;
                }

                match solver.check_sat()? {
                    SatResult::Sat => {
                        let trace = solver.extract_trace(m, k + 1)?;
                        solver.pop()?;
                        return Ok(LiveOutcome::Fail {
                            property: m.properties[live_idx].name.clone(),
                            loop_start: l,
                            loop_end: k,
                            trace,
                        });
                    }
                    SatResult::Unknown => {
                        solver.pop()?;
                        return Ok(LiveOutcome::Unknown { step: k });
                    }
                    SatResult::Unsat => {
                        solver.pop()?;
                    }
                }
            }
        }
    }

    Ok(LiveOutcome::Pass { depth: cfg.depth })
}
