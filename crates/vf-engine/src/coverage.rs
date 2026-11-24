use anyhow::Result;
use std::collections::HashSet;
use std::thread;
use vf_ir::{ExprKind, Module, PropertyKind, SignalKind};

use crate::bmc::{run_bmc, BmcConfig, BmcOutcome};
use crate::kind::{run_kinduction, ProveConfig, ProveOutcome};
use crate::portfolio::{run_portfolio, PortfolioConfig};

#[derive(Debug, Clone)]
pub struct AssumptionCoreItem {
    pub name: String,
    /// `true` if removing this assumption causes the proof to fail or become unknown;
    /// `false` if the proof still holds without it (redundant / potential over-constraint).
    pub essential: bool,
}

#[derive(Debug, Clone)]
pub struct ProofCoreReport {
    pub total_signals: usize,
    pub coi_signals: usize,
    pub coi_signal_names: Vec<String>,
    pub uncovered_signal_names: Vec<String>,
    pub assumptions: Vec<AssumptionCoreItem>,
}

/// JasperGold Proof Core / Questa Formal Coverage equivalent:
/// 1. Computes exact Cone-of-Influence (COI) signal coverage for the proved assertions.
/// 2. Computes the minimal assumption core by testing each `Assume` individually to flag
///    redundant/unnecessary assumptions (over-constraint detection).
pub fn compute_proof_core(m: &Module, cfg: &PortfolioConfig) -> Result<ProofCoreReport> {
    let mut coi = m.clone();
    vf_ir::passes::coi_prune(&mut coi);

    let coi_driven: HashSet<_> = coi
        .comb
        .iter()
        .map(|c| c.signal)
        .chain(coi.ffs.iter().map(|f| f.signal))
        .collect();

    let mut coi_names = Vec::new();
    let mut uncov_names = Vec::new();
    for s in &m.signals {
        if s.name.starts_with('$') || s.kind == SignalKind::Input || s.is_clock {
            continue;
        }
        if coi_driven.contains(&s.id) {
            coi_names.push(s.name.clone());
        } else {
            uncov_names.push(s.name.clone());
        }
    }

    // Minimal assumption core: for each Assume property, drop it and test whether proof still holds.
    let assume_indices: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Assume)
        .map(|(i, _)| i)
        .collect();

    let mut assumptions = Vec::with_capacity(assume_indices.len());
    for idx in assume_indices {
        let name = m.properties[idx].name.clone();
        let mut without = m.clone();
        without.properties.remove(idx);
        let res = run_portfolio(&without, cfg)?;
        let essential = !matches!(res.outcome, ProveOutcome::Proved { .. });
        assumptions.push(AssumptionCoreItem { name, essential });
    }

    Ok(ProofCoreReport {
        total_signals: coi_names.len() + uncov_names.len(),
        coi_signals: coi_names.len(),
        coi_signal_names: coi_names,
        uncovered_signal_names: uncov_names,
        assumptions,
    })
}

#[derive(Debug, Clone)]
pub struct MutationConfig {
    pub depth: usize,
    pub max_mutants: usize,
    pub solver: String,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            depth: 12,
            max_mutants: 32,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutantStatus {
    /// At least one assertion caught the injected RTL bug!
    Killed,
    /// All assertions STILL passed despite the RTL mutation (formal spec gap!).
    Survived,
}

#[derive(Debug, Clone)]
pub struct MutantResult {
    pub id: usize,
    pub description: String,
    pub status: MutantStatus,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct MutationReport {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub score_percent: f64,
    pub mutants: Vec<MutantResult>,
}

/// Formal Mutation Testing (`veloformal mutate`):
/// Injects targeted RTL bugs directly into the COI of the design (flipped comparisons, inverted
/// mux guards, Add<->Sub, stuck-at-0/1 drivers) and tests whether the formal assertions kill
/// each mutant. Returns an exact Formal Verification Completeness Score (%).
pub fn run_mutation_analysis(base: &Module, cfg: &MutationConfig) -> Result<MutationReport> {
    let mut coi = base.clone();
    vf_ir::passes::coi_prune(&mut coi);

    let mut candidates: Vec<(String, Module)> = Vec::new();

    // 1. Expression operator mutations inside COI expressions used by FFs and comb assigns.
    let mut mut_expr_ids: Vec<usize> = Vec::new();
    for (idx, node) in coi.exprs.iter().enumerate() {
        let mutable = matches!(
            node.kind,
            ExprKind::Ult(_, _)
                | ExprKind::Ule(_, _)
                | ExprKind::Slt(_, _)
                | ExprKind::Sle(_, _)
                | ExprKind::Add(_, _)
                | ExprKind::Sub(_, _)
                | ExprKind::Mux { .. }
        );
        if mutable {
            mut_expr_ids.push(idx);
        }
    }

    for idx in mut_expr_ids {
        if candidates.len() >= cfg.max_mutants {
            break;
        }
        let mut mutated = coi.clone();
        let desc = match mutated.exprs[idx].kind.clone() {
            ExprKind::Ult(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Ule(a, b);
                format!("expr#{idx}: flip `<` to `<=`")
            }
            ExprKind::Ule(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Ult(a, b);
                format!("expr#{idx}: flip `<=` to `<`")
            }
            ExprKind::Slt(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Sle(a, b);
                format!("expr#{idx}: flip signed `<` to `<=`")
            }
            ExprKind::Sle(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Slt(a, b);
                format!("expr#{idx}: flip signed `<=` to `<`")
            }
            ExprKind::Add(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Sub(a, b);
                format!("expr#{idx}: flip `+` to `-`")
            }
            ExprKind::Sub(a, b) => {
                mutated.exprs[idx].kind = ExprKind::Add(a, b);
                format!("expr#{idx}: flip `-` to `+`")
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                mutated.exprs[idx].kind = ExprKind::Mux {
                    cond,
                    then_val: else_val,
                    else_val: then_val,
                };
                format!("expr#{idx}: invert mux branches")
            }
            _ => continue,
        };
        candidates.push((desc, mutated));
    }

    // 2. Stuck-at-0 FF next-state mutations for non-internal FFs in COI.
    for fi in 0..coi.ffs.len() {
        if candidates.len() >= cfg.max_mutants {
            break;
        }
        let sname = coi.signal(coi.ffs[fi].signal).name.clone();
        if sname.starts_with('$') {
            continue;
        }
        let w = coi.signal(coi.ffs[fi].signal).width;
        let mut mutated = coi.clone();
        let z = mutated.c_u64(w, 0);
        mutated.ffs[fi].next = z;
        candidates.push((format!("ff `{sname}`: stuck-at-0 next-state"), mutated));
    }

    // Evaluate mutants in parallel batches (up to 4 threads).
    let depth = cfg.depth;
    let solver = cfg.solver.clone();
    let mut results = Vec::with_capacity(candidates.len());
    for chunk in candidates.chunks(4).enumerate() {
        let (chunk_idx, slice) = chunk;
        let handles: Vec<_> = slice
            .iter()
            .enumerate()
            .map(|(j, (desc, m))| {
                let id = chunk_idx * 4 + j;
                let desc = desc.clone();
                let m = m.clone();
                let s = solver.clone();
                thread::spawn(move || {
                    // Fast BMC first (catches almost all killed mutants in milliseconds),
                    // then short k-induction check if BMC passes.
                    let bmc = run_bmc(
                        &m,
                        &BmcConfig {
                            depth,
                            solver: s.clone(),
                            skip: 0,
                            append: 0,
                        },
                    );
                    match bmc {
                        Ok(BmcOutcome::Fail { property, step, .. }) => MutantResult {
                            id,
                            description: desc,
                            status: MutantStatus::Killed,
                            detail: format!("killed by `{property}` at cycle {step}"),
                        },
                        _ => {
                            let kind = run_kinduction(
                                &m,
                                &ProveConfig {
                                    max_k: depth.min(4),
                                    solver: s,
                                },
                            );
                            match kind {
                                Ok(ProveOutcome::Fail { property, step, .. }) => MutantResult {
                                    id,
                                    description: desc,
                                    status: MutantStatus::Killed,
                                    detail: format!("killed by `{property}` at cycle {step}"),
                                },
                                _ => MutantResult {
                                    id,
                                    description: desc,
                                    status: MutantStatus::Survived,
                                    detail: format!(
                                        "survived BMC depth {depth} (potential spec coverage gap)"
                                    ),
                                },
                            }
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            if let Ok(r) = h.join() {
                results.push(r);
            }
        }
    }

    let total = results.len();
    let killed = results
        .iter()
        .filter(|r| r.status == MutantStatus::Killed)
        .count();
    let survived = total.saturating_sub(killed);
    let score_percent = if total == 0 {
        100.0
    } else {
        (killed as f64 / total as f64) * 100.0
    };

    Ok(MutationReport {
        total,
        killed,
        survived,
        score_percent,
        mutants: results,
    })
}
