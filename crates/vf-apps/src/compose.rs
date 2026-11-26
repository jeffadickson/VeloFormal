use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use vf_dsl::{lower_contract_to_ir, lower_contract_to_ir_for_instance, parse_contract, ContractRole};
use vf_engine::kind::ProveOutcome;
use vf_engine::portfolio::{run_portfolio, PortfolioConfig};
use vf_ir::passes::{async2sync, blackbox_instance, clk2fflogic, has_multiple_clocks};
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::solver::TraceFrame;

/// One submodule leaf in an assume-guarantee hierarchical proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleSpec {
    /// Instance name inside the top-level RTL (e.g. `"u_mul"` or `"u_crypto"`).
    pub instance: String,
    /// Optional explicit top module name when compiling the submodule standalone.
    #[serde(default)]
    pub module_name: Option<String>,
    /// SystemVerilog source file(s) for the submodule standalone proof.
    pub rtl_files: Vec<PathBuf>,
    /// Path to the submodule's `.vfc` v1.0 contract.
    pub spec_file: PathBuf,
    /// Optional `-G` contract parameters.
    #[serde(default)]
    pub params: HashMap<String, i64>,
}

/// Complete specification for automated hierarchical assume-guarantee composition (`veloformal compose`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// Top-level SystemVerilog source file(s) (must include submodule definitions so Slang elaborates ports).
    pub top_rtl_files: Vec<PathBuf>,
    /// Optional top-level module name.
    #[serde(default)]
    pub top_module: Option<String>,
    /// Top-level `.vfc` contract file.
    pub top_spec_file: PathBuf,
    /// Optional `-G` parameters for the top-level contract.
    #[serde(default)]
    pub top_params: HashMap<String, i64>,
    /// Submodules to verify standalone (`Provider`), blackbox in top, and bind in `Caller` role.
    pub submodules: Vec<SubmoduleSpec>,
    #[serde(default = "default_depth")]
    pub depth: usize,
    #[serde(default = "default_solver")]
    pub solver: String,
    #[serde(default = "default_true")]
    pub enable_pdr: bool,
}

fn default_depth() -> usize {
    20
}
fn default_solver() -> String {
    "z3".to_string()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct LeafProofResult {
    pub instance: String,
    pub contract_name: String,
    pub original_ffs: usize,
    pub outcome: ProveOutcome,
    pub winner: String,
}

#[derive(Debug, Clone)]
pub struct ComposeResult {
    pub top_module_name: String,
    pub top_contract_name: String,
    pub leaf_results: Vec<LeafProofResult>,
    pub top_ffs_before_blackbox: usize,
    pub top_ffs_after_blackbox: usize,
    pub top_blackboxed_signals: usize,
    pub top_outcome: ProveOutcome,
    pub top_winner: String,
    pub composed_proved: bool,
    pub top_module: Module,
    pub failure_trace: Option<Vec<TraceFrame>>,
}

/// Parse a CLI `--sub inst=rtl.sv:spec.vfc` (or `inst=rtl.sv:mod_name:spec.vfc`) specification.
pub fn parse_sub_arg(raw: &str) -> Result<SubmoduleSpec> {
    let (inst, rest) = raw
        .split_once('=')
        .ok_or_else(|| anyhow!("invalid --sub '{raw}': expected `instance=rtl.sv:spec.vfc`"))?;
    let parts: Vec<&str> = rest.split(':').collect();
    match parts.as_slice() {
        [rtl, spec] => Ok(SubmoduleSpec {
            instance: inst.trim().to_string(),
            module_name: None,
            rtl_files: rtl
                .split(',')
                .map(|s| PathBuf::from(s.trim()))
                .collect(),
            spec_file: PathBuf::from(spec.trim()),
            params: HashMap::new(),
        }),
        [rtl, mod_name, spec] => Ok(SubmoduleSpec {
            instance: inst.trim().to_string(),
            module_name: Some(mod_name.trim().to_string()),
            rtl_files: rtl
                .split(',')
                .map(|s| PathBuf::from(s.trim()))
                .collect(),
            spec_file: PathBuf::from(spec.trim()),
            params: HashMap::new(),
        }),
        _ => bail!("invalid --sub '{raw}': expected `instance=rtl.sv:spec.vfc` or `instance=rtl.sv:top_mod:spec.vfc`"),
    }
}

fn prepare_ir(mut m: Module) -> Module {
    if has_multiple_clocks(&m) {
        clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        async2sync(&mut m);
    }
    m
}

/// Execute sound two-phase Hierarchical Assume-Guarantee Composition:
/// 1. **Phase 1 (Leaf Provider Proofs)**: Compile each submodule standalone and prove its `.vfc`
///    contract in `ContractRole::Provider` (assumes preconditions, proves all invariants/ensures).
/// 2. **Phase 2 (Top-Level Abstracted Proof)**: Blackbox every verified submodule instance in the
///    top-level RTL (`blackbox_instance`), bind each submodule's `.vfc` contract in `ContractRole::Caller`
///    (turning proved guarantees into assumptions AND preconditions into top-level proof obligations
///    for zero-circularity soundness), bind the top-level `.vfc` contract in `Provider` role, and prove!
pub fn run_compose(cfg: &ComposeConfig) -> Result<ComposeResult> {
    let pcfg = PortfolioConfig {
        depth: cfg.depth,
        solver: cfg.solver.clone(),
        enable_pdr: cfg.enable_pdr,
        cache_dir: None,
    };

    // Phase 1: Verify every submodule leaf in Provider role.
    let mut leaf_results = Vec::with_capacity(cfg.submodules.len());
    let mut parsed_sub_contracts = Vec::with_capacity(cfg.submodules.len());

    for sub in &cfg.submodules {
        let spec_text = fs::read_to_string(&sub.spec_file)
            .with_context(|| format!("failed to read submodule spec {}", sub.spec_file.display()))?;
        let contract = parse_contract(&spec_text)
            .with_context(|| format!("failed to parse submodule spec {}", sub.spec_file.display()))?;

        let mut opts = CompileOptions::for_files(sub.rtl_files.clone());
        opts.top = sub
            .module_name
            .clone()
            .or_else(|| Some(contract.target_module.clone()));
        let base = prepare_ir(compile_to_ir(&opts)?);
        let original_ffs = base.ffs.len();
        let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &sub.params)?;
        let pr = run_portfolio(&lowered.module, &pcfg)?;

        let leaf_failed = !matches!(pr.outcome, ProveOutcome::Proved { .. });
        let fail_trace = match &pr.outcome {
            ProveOutcome::Fail { trace, .. } => Some(trace.clone()),
            _ => None,
        };
        leaf_results.push(LeafProofResult {
            instance: sub.instance.clone(),
            contract_name: contract.name.clone(),
            original_ffs,
            outcome: pr.outcome.clone(),
            winner: pr.winner,
        });
        parsed_sub_contracts.push((sub, contract));

        if leaf_failed {
            return Ok(ComposeResult {
                top_module_name: cfg.top_module.clone().unwrap_or_default(),
                top_contract_name: String::new(),
                leaf_results,
                top_ffs_before_blackbox: 0,
                top_ffs_after_blackbox: 0,
                top_blackboxed_signals: 0,
                top_outcome: pr.outcome,
                top_winner: "aborted(leaf_failed)".into(),
                composed_proved: false,
                top_module: lowered.module,
                failure_trace: fail_trace,
            });
        }
    }

    // Phase 2: Compile top-level SoC, blackbox verified submodules, bind Caller contracts + top Provider contract.
    let top_spec_text = fs::read_to_string(&cfg.top_spec_file)
        .with_context(|| format!("failed to read top spec {}", cfg.top_spec_file.display()))?;
    let top_contract = parse_contract(&top_spec_text)
        .with_context(|| format!("failed to parse top spec {}", cfg.top_spec_file.display()))?;

    let mut top_opts = CompileOptions::for_files(cfg.top_rtl_files.clone());
    top_opts.top = cfg
        .top_module
        .clone()
        .or_else(|| Some(top_contract.target_module.clone()));
    let mut top_mod = prepare_ir(compile_to_ir(&top_opts)?);
    let top_ffs_before_blackbox = top_mod.ffs.len();

    let mut total_blackboxed = 0usize;
    for (sub, sub_contract) in &parsed_sub_contracts {
        let removed = blackbox_instance(&mut top_mod, &sub.instance, false);
        if removed == 0 {
            bail!(
                "instance '{}' not found or had no internal signals to blackbox in top module '{}'",
                sub.instance,
                top_mod.name
            );
        }
        total_blackboxed += removed;
        let bound = lower_contract_to_ir_for_instance(
            &top_mod,
            sub_contract,
            ContractRole::Caller,
            &sub.params,
            Some(&sub.instance),
        )?;
        top_mod = bound.module;
    }

    let top_ffs_after_blackbox = top_mod.ffs.len();
    let top_lowered = lower_contract_to_ir(
        &top_mod,
        &top_contract,
        ContractRole::Provider,
        &cfg.top_params,
    )?;
    let top_pr = run_portfolio(&top_lowered.module, &pcfg)?;
    let composed_proved = matches!(top_pr.outcome, ProveOutcome::Proved { .. });
    let failure_trace = match &top_pr.outcome {
        ProveOutcome::Fail { trace, .. } => Some(trace.clone()),
        _ => None,
    };

    Ok(ComposeResult {
        top_module_name: top_lowered.module.name.clone(),
        top_contract_name: top_contract.name,
        leaf_results,
        top_ffs_before_blackbox,
        top_ffs_after_blackbox,
        top_blackboxed_signals: total_blackboxed,
        top_outcome: top_pr.outcome,
        top_winner: top_pr.winner,
        composed_proved,
        top_module: top_lowered.module,
        failure_trace,
    })
}
