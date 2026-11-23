use anyhow::Result;
use vf_ir::{Module, PropertyKind};
use vf_smt::smt2;
use vf_smt::solver::{SatResult, SmtSolver, TraceFrame};

#[derive(Debug, Clone)]
pub struct CoverConfig {
    pub depth: usize,
    pub solver: String,
}

impl Default for CoverConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverResult {
    pub name: String,
    pub reached: Option<(usize, Vec<TraceFrame>)>,
}

pub fn run_cover(m: &Module, cfg: &CoverConfig) -> Result<Vec<CoverResult>> {
    let cover_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Cover)
        .map(|(i, _)| i)
        .collect();

    let mut results: Vec<CoverResult> = cover_indices
        .iter()
        .map(|&i| CoverResult {
            name: m.properties[i].name.clone(),
            reached: None,
        })
        .collect();

    if cover_indices.is_empty() {
        return Ok(results);
    }

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send(&smt2::header(m))?;

    for t in 0..cfg.depth {
        solver.send(&smt2::encode_step(m, t, t == 0, t.checked_sub(1)))?;
        solver.send(&smt2::encode_assumes(m, t))?;

        let mut all_reached = true;
        for (slot, &idx) in cover_indices.iter().enumerate() {
            if results[slot].reached.is_some() {
                continue;
            }
            all_reached = false;
            solver.push()?;
            let (defs, hit) = smt2::encode_cover_hit(m, idx, t);
            solver.send(&defs)?;
            solver.send(&format!("(assert {hit})\n"))?;
            if solver.check_sat()? == SatResult::Sat {
                let trace = solver.extract_trace(m, t + 1)?;
                results[slot].reached = Some((t, trace));
            }
            solver.pop()?;
        }

        if all_reached || results.iter().all(|r| r.reached.is_some()) {
            break;
        }
    }

    Ok(results)
}
