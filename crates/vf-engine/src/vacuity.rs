use anyhow::Result;
use vf_ir::{Module, Property, PropertyKind};
use vf_smt::solver::TraceFrame;

use crate::cover::{run_cover, CoverConfig};
use crate::kind::{run_kinduction, ProveConfig, ProveOutcome};
use crate::pdr::{run_pdr, PdrConfig};

#[derive(Debug, Clone)]
pub struct VacuityConfig {
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for VacuityConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum VacuityStatus {
    /// Antecedent/guard was reached at cycle `step` — assertion is NOT vacuous.
    NonVacuous { step: usize, trace: Vec<TraceFrame> },
    /// Proved unboundedly that antecedent/guard can NEVER be true — assertion is VACUOUS!
    Vacuous,
    /// Antecedent not reached within `depth` cycles, and unreachability was not proved.
    BoundedVacuous { depth: usize },
}

#[derive(Debug, Clone)]
pub struct VacuityResult {
    pub property: String,
    pub status: VacuityStatus,
}

/// Questa PropCheck vacuity analysis: for each `Assert` property, verifies whether its
/// enable/antecedent (`p.en`) is actually reachable. Flags assertions that only "pass"
/// because their trigger/guard is never satisfied!
pub fn check_vacuity(m: &Module, cfg: &VacuityConfig) -> Result<Vec<VacuityResult>> {
    let asserts: Vec<Property> = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assert)
        .cloned()
        .collect();

    if asserts.is_empty() {
        return Ok(Vec::new());
    }

    // Step 1: Build a cover model with one Cover goal per assertion's enable guard `p.en`.
    let mut cov_mod = m.clone();
    cov_mod.properties.retain(|p| p.kind == PropertyKind::Assume);
    let one = cov_mod.c_bool(true);
    for a in &asserts {
        cov_mod.properties.push(Property {
            name: a.name.clone(),
            kind: PropertyKind::Cover,
            cond: a.en,
            en: one,
            source: a.source.clone(),
        });
    }

    let cov_res = run_cover(
        &cov_mod,
        &CoverConfig {
            depth: cfg.depth,
            solver: cfg.solver.clone(),
            append: 0,
        },
    )?;

    let mut results = Vec::with_capacity(asserts.len());
    for (a, cr) in asserts.iter().zip(cov_res) {
        if let Some((step, trace)) = cr.reached {
            results.push(VacuityResult {
                property: a.name.clone(),
                status: VacuityStatus::NonVacuous { step, trace },
            });
            continue;
        }

        // Step 2: Not hit within `depth` — try to PROVE `assert(!a.en)` unboundedly (proving vacuity!).
        let mut vac_mod = m.clone();
        vac_mod.properties.retain(|p| p.kind == PropertyKind::Assume);
        let one_v = vac_mod.c_bool(true);
        let not_en = vac_mod.logic_not(a.en);
        vac_mod.properties.push(Property {
            name: format!("vacuity_{}", a.name),
            kind: PropertyKind::Assert,
            cond: not_en,
            en: one_v,
            source: a.source.clone(),
        });
        vf_ir::passes::coi_prune(&mut vac_mod);

        let proved = match run_kinduction(
            &vac_mod,
            &ProveConfig {
                max_k: cfg.depth.min(8),
                solver: cfg.solver.clone(),
            },
        )? {
            ProveOutcome::Proved { .. } => true,
            ProveOutcome::Unknown { .. } if cfg.use_pdr => matches!(
                run_pdr(
                    &vac_mod,
                    &PdrConfig {
                        smt_solver: cfg.solver.clone(),
                        ..Default::default()
                    },
                ),
                Ok(ProveOutcome::Proved { .. })
            ),
            _ => false,
        };

        results.push(VacuityResult {
            property: a.name.clone(),
            status: if proved {
                VacuityStatus::Vacuous
            } else {
                VacuityStatus::BoundedVacuous { depth: cfg.depth }
            },
        });
    }

    Ok(results)
}
