use anyhow::Result;
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{Module, Property, PropertyKind};
use vf_slang::{compile_to_ir_with_options, CompileOptions, InstrumentOptions};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct CoverCheckConfig {
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for CoverCheckConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CoverageStatus {
    /// Branch is reachable (witness trace found at `step`).
    Reachable { step: usize, trace: Vec<TraceFrame> },
    /// Formally proved UNREACHABLE for all time (safe dead-code waiver!).
    Unreachable,
    /// Not reached within `depth`, and unreachability was not proved.
    Undecided,
}

#[derive(Debug, Clone)]
pub struct CoverageItem {
    pub name: String,
    pub status: CoverageStatus,
}

/// Questa CoverCheck equivalent: automatically instruments every RTL `if`/`else`/`case` branch,
/// finds witnesses for reachable branches, and formally proves unreachable dead-code branches
/// using k-induction + ABC PDR.
pub fn run_covercheck(
    compile: &CompileOptions,
    cfg: &CoverCheckConfig,
) -> Result<(Module, Vec<CoverageItem>)> {
    let inst = InstrumentOptions {
        branch_covers: true,
        ..Default::default()
    };
    let mut m = compile_to_ir_with_options(compile, inst)?;
    if vf_ir::passes::has_multiple_clocks(&m) {
        vf_ir::passes::clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        vf_ir::passes::async2sync(&mut m);
    }

    let branch_props: Vec<Property> = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Cover && p.name.starts_with("branch_"))
        .cloned()
        .collect();

    if branch_props.is_empty() {
        return Ok((m, Vec::new()));
    }

    // Step 1: Fast cover search for all branches simultaneously.
    let mut cov_mod = m.clone();
    cov_mod.properties.retain(|p| p.kind == PropertyKind::Assume);
    cov_mod.properties.extend(branch_props.iter().cloned());

    let cover_results = run_cover(
        &cov_mod,
        &CoverConfig {
            depth: cfg.depth,
            solver: cfg.solver.clone(),
            append: 0,
        },
    )?;

    let mut items = Vec::with_capacity(branch_props.len());
    for (prop, cr) in branch_props.iter().zip(cover_results) {
        if let Some((step, trace)) = cr.reached {
            items.push(CoverageItem {
                name: prop.name.clone(),
                status: CoverageStatus::Reachable { step, trace },
            });
            continue;
        }

        // Step 2: Not hit in BMC — attempt unbounded unreachability proof: `assert(!(en && cond))`!
        let mut unreach_mod = m.clone();
        unreach_mod.properties.retain(|p| p.kind == PropertyKind::Assume);
        let hit = unreach_mod.logic_and(prop.en, prop.cond);
        let never_hit = unreach_mod.logic_not(hit);
        let one = unreach_mod.c_bool(true);
        unreach_mod.properties.push(Property {
            name: format!("unreach_{}", prop.name),
            kind: PropertyKind::Assert,
            cond: never_hit,
            en: one,
            source: prop.source.clone(),
        });
        vf_ir::passes::coi_prune(&mut unreach_mod);

        let proved_unreachable = match run_kinduction(
            &unreach_mod,
            &ProveConfig {
                max_k: cfg.depth.min(6),
                solver: cfg.solver.clone(),
            },
        )? {
            ProveOutcome::Proved { .. } => true,
            ProveOutcome::Unknown { .. } if cfg.use_pdr => matches!(
                run_pdr(
                    &unreach_mod,
                    &PdrConfig {
                        smt_solver: cfg.solver.clone(),
                        ..Default::default()
                    },
                ),
                Ok(ProveOutcome::Proved { .. })
            ),
            _ => false,
        };

        items.push(CoverageItem {
            name: prop.name.clone(),
            status: if proved_unreachable {
                CoverageStatus::Unreachable
            } else {
                CoverageStatus::Undecided
            },
        });
    }

    Ok((m, items))
}
