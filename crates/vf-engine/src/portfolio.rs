use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use vf_ir::{Module, Property, PropertyKind};
use vf_smt::solver::CancelToken;

use crate::cache::{module_fingerprint, CachedVerdict, ProofCache};
use crate::kind::{run_kinduction_cancellable, ProveConfig, ProveOutcome};
use crate::pdr::{run_pdr_cancellable, PdrConfig, PdrStrategy};

#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    pub depth: usize,
    pub solver: String,
    pub enable_pdr: bool,
    pub cache_dir: Option<PathBuf>,
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
            enable_pdr: true,
            cache_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortfolioResult {
    pub outcome: ProveOutcome,
    pub winner: String,
    pub cache_hit: bool,
}

/// Prepare a module for safety proof: Cone-of-Influence pruning followed by sound sequential
/// constant propagation / duplicate-FF sweeping and a second COI prune to drop newly-dead aliases.
fn prepare_for_proof(module: &Module) -> Module {
    let mut m = module.clone();
    vf_ir::passes::coi_prune(&mut m);
    vf_ir::opt::sweep_sequential(&mut m);
    vf_ir::passes::coi_prune(&mut m);
    m
}

/// True parallel multi-engine portfolio orchestrator with automatic per-property COI decomposition:
/// - When a module has multiple assertions, decomposes each assertion onto its minimal COI and
///   proves them concurrently across CPU cores (promoting proved assertions as sound helper lemmas
///   if any assertion requires monolithic induction).
/// - For each proof obligation, races word-level k-induction, ABC `dc2+scorr+pdr`, and ABC
///   `dc2+int` Craig interpolation concurrently, cancelling all losing subprocesses in < 1 ms.
/// - Supports content-addressed COI proof caching.
pub fn run_portfolio(module: &Module, cfg: &PortfolioConfig) -> Result<PortfolioResult> {
    let assert_count = module
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assert)
        .count();
    if assert_count > 1 {
        run_portfolio_decomposed(module, cfg)
    } else {
        run_portfolio_monolithic(module, cfg)
    }
}

/// Per-property parallel COI decomposition with fail-fast abort and sound lemma promotion fallback.
pub fn run_portfolio_decomposed(module: &Module, cfg: &PortfolioConfig) -> Result<PortfolioResult> {
    let assert_indices: Vec<usize> = module
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Assert)
        .map(|(i, _)| i)
        .collect();

    if assert_indices.is_empty() {
        return Ok(PortfolioResult {
            outcome: ProveOutcome::Proved { k: 0 },
            winner: "trivial(0 asserts)".into(),
            cache_hit: false,
        });
    }
    if assert_indices.len() == 1 {
        return run_portfolio_monolithic(module, cfg);
    }

    // Check whole-module COI proof cache first so repeated full-module proofs hit instantaneously.
    let whole_pruned = prepare_for_proof(module);
    let whole_cache_key = module_fingerprint(&whole_pruned, "portfolio-prove");
    if let Some(dir) = &cfg.cache_dir {
        let cache = ProofCache::open(dir);
        if let Some(hit) = cache.get(&whole_cache_key) {
            return Ok(PortfolioResult {
                outcome: ProveOutcome::Proved { k: hit.k },
                winner: format!("cache({})", hit.engine),
                cache_hit: true,
            });
        }
    }

    // Build minimal per-property sliced sub-modules (keeping all Assumes + single Assert).
    let slices: Vec<(usize, Module)> = assert_indices
        .iter()
        .map(|&idx| {
            let mut sub = module.clone();
            let target = module.properties[idx].clone();
            sub.properties.retain(|p| p.kind == PropertyKind::Assume);
            sub.properties.push(target);
            (idx, sub)
        })
        .collect();

    let n_props = slices.len();
    let max_workers = thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(4)
        .min(n_props);

    let next_idx = AtomicUsize::new(0);
    let global_abort = Arc::new(AtomicBool::new(false));
    let active_tokens: Arc<Mutex<Vec<Arc<CancelToken>>>> = Arc::new(Mutex::new(Vec::new()));
    let results: Mutex<Vec<(usize, Result<PortfolioResult>)>> =
        Mutex::new(Vec::with_capacity(n_props));

    let cancel_all = |tokens: &Arc<Mutex<Vec<Arc<CancelToken>>>>| {
        if let Ok(list) = tokens.lock() {
            for t in list.iter() {
                t.cancel();
            }
        }
    };

    thread::scope(|scope| {
        for _ in 0..max_workers {
            let next_ref = &next_idx;
            let slices_ref = &slices;
            let results_ref = &results;
            let abort_ref = Arc::clone(&global_abort);
            let tokens_ref = Arc::clone(&active_tokens);
            let cfg_clone = cfg.clone();

            scope.spawn(move || loop {
                if abort_ref.load(Ordering::SeqCst) {
                    break;
                }
                let slot = next_ref.fetch_add(1, Ordering::Relaxed);
                if slot >= slices_ref.len() {
                    break;
                }
                let (prop_idx, ref sub_mod) = slices_ref[slot];
                let prop_cancel = CancelToken::new();
                if let Ok(mut list) = tokens_ref.lock() {
                    if abort_ref.load(Ordering::SeqCst) {
                        prop_cancel.cancel();
                        break;
                    }
                    list.push(Arc::clone(&prop_cancel));
                }

                let res = run_portfolio_monolithic_with_cancel(sub_mod, &cfg_clone, prop_cancel);
                let is_fatal = matches!(
                    res,
                    Ok(PortfolioResult {
                        outcome: ProveOutcome::Fail { .. } | ProveOutcome::AssumptionsUnsat { .. },
                        ..
                    })
                );
                if is_fatal {
                    abort_ref.store(true, Ordering::SeqCst);
                    cancel_all(&tokens_ref);
                }
                if let Ok(mut out) = results_ref.lock() {
                    out.push((prop_idx, res));
                }
                if is_fatal {
                    break;
                }
            });
        }
    });

    // Ensure all worker subprocesses are stopped.
    cancel_all(&active_tokens);

    let collected = results.into_inner().unwrap_or_default();
    let mut proved_indices: HashSet<usize> = HashSet::new();
    let mut unknown_indices: Vec<usize> = Vec::new();
    let mut max_k = 0usize;
    let mut all_cache_hit = !collected.is_empty();
    let mut last_err: Option<anyhow::Error> = None;

    for (idx, res) in collected {
        match res {
            Ok(pr) => match pr.outcome {
                ProveOutcome::Fail { .. } | ProveOutcome::AssumptionsUnsat { .. } => {
                    return Ok(pr);
                }
                ProveOutcome::Proved { k } => {
                    proved_indices.insert(idx);
                    max_k = max_k.max(k);
                    all_cache_hit &= pr.cache_hit;
                }
                ProveOutcome::Unknown { .. } => {
                    unknown_indices.push(idx);
                    all_cache_hit = false;
                }
            },
            Err(e) => {
                unknown_indices.push(idx);
                all_cache_hit = false;
                last_err = Some(e);
            }
        }
    }

    // Account for any assertions skipped due to error/unknown.
    for &idx in &assert_indices {
        if !proved_indices.contains(&idx) && !unknown_indices.contains(&idx) {
            unknown_indices.push(idx);
        }
    }

    if unknown_indices.is_empty() {
        let winner = format!("decomposed({n_props} props)");
        if let Some(dir) = &cfg.cache_dir {
            let mut cache = ProofCache::open(dir);
            let _ = cache.insert(
                whole_cache_key,
                CachedVerdict {
                    engine: winner.clone(),
                    k: max_k,
                },
            );
        }
        return Ok(PortfolioResult {
            outcome: ProveOutcome::Proved { k: max_k },
            winner,
            cache_hit: all_cache_hit,
        });
    }

    // Sound lemma-promotion fallback: any assertions already proved independently are valid
    // invariants and may be promoted to `Assume` to assist monolithic k-induction/PDR on the
    // remaining assertions (handles mutual inductive helper lemmas with zero soundness loss).
    let mut fallback_mod = module.clone();
    let unknown_set: HashSet<usize> = unknown_indices.iter().copied().collect();
    let mut new_props = Vec::new();
    for (idx, p) in module.properties.iter().enumerate() {
        match p.kind {
            PropertyKind::Assume => new_props.push(p.clone()),
            PropertyKind::Assert if proved_indices.contains(&idx) => {
                let mut lemma = p.clone();
                lemma.kind = PropertyKind::Assume;
                new_props.push(lemma);
            }
            PropertyKind::Assert if unknown_set.contains(&idx) => {
                new_props.push(p.clone());
            }
            _ => {}
        }
    }
    fallback_mod.properties = new_props;

    match run_portfolio_monolithic(&fallback_mod, cfg) {
        Ok(pr) => {
            if let ProveOutcome::Proved { k } = pr.outcome {
                let winner = format!("decomposed+lemma({})", pr.winner);
                if let Some(dir) = &cfg.cache_dir {
                    let mut cache = ProofCache::open(dir);
                    let _ = cache.insert(
                        whole_cache_key,
                        CachedVerdict {
                            engine: winner.clone(),
                            k: k.max(max_k),
                        },
                    );
                }
                return Ok(PortfolioResult {
                    outcome: ProveOutcome::Proved { k: k.max(max_k) },
                    winner,
                    cache_hit: false,
                });
            }
            Ok(pr)
        }
        Err(e) => Err(last_err.unwrap_or(e)),
    }
}

/// Monolithic multi-engine parallel portfolio on a single COI-pruned & sequentially-swept module.
/// Races word-level k-induction, ABC `strash; dc2; scorr; dc2; pdr`, and ABC `strash; dc2; int -k`
/// concurrently, immediately cancelling and reaping all losing solver subprocesses on the first verdict.
pub fn run_portfolio_monolithic(module: &Module, cfg: &PortfolioConfig) -> Result<PortfolioResult> {
    run_portfolio_monolithic_with_cancel(module, cfg, CancelToken::new())
}

fn run_portfolio_monolithic_with_cancel(
    module: &Module,
    cfg: &PortfolioConfig,
    cancel: Arc<CancelToken>,
) -> Result<PortfolioResult> {
    if cancel.is_cancelled() {
        return Ok(PortfolioResult {
            outcome: ProveOutcome::Unknown { max_k: 0 },
            winner: "cancelled".into(),
            cache_hit: false,
        });
    }

    let pruned = prepare_for_proof(module);

    let cache_key = module_fingerprint(&pruned, "portfolio-prove");
    if let Some(dir) = &cfg.cache_dir {
        let cache = ProofCache::open(dir);
        if let Some(hit) = cache.get(&cache_key) {
            return Ok(PortfolioResult {
                outcome: ProveOutcome::Proved { k: hit.k },
                winner: format!("cache({})", hit.engine),
                cache_hit: true,
            });
        }
    }

    let (tx, rx) = mpsc::channel::<(String, Result<ProveOutcome>)>();
    let mut launched = 0usize;

    // Worker 1: word-level k-induction with configured SMT solver (default Z3)
    {
        let m = pruned.clone();
        let tx1 = tx.clone();
        let depth = cfg.depth;
        let solver = cfg.solver.clone();
        let tok = Arc::clone(&cancel);
        launched += 1;
        thread::spawn(move || {
            let res = run_kinduction_cancellable(
                &m,
                &ProveConfig {
                    max_k: depth,
                    solver: solver.clone(),
                },
                Some(tok),
            );
            let _ = tx1.send((format!("kind({solver})"), res));
        });
    }

    // Worker 2: IC3/PDR with AIG pre-simplification (`strash; dc2; scorr -C 500; dc2; pdr`)
    if cfg.enable_pdr {
        let m = pruned.clone();
        let tx2 = tx.clone();
        let solver = cfg.solver.clone();
        let tok = Arc::clone(&cancel);
        launched += 1;
        thread::spawn(move || {
            let res = run_pdr_cancellable(
                &m,
                &PdrConfig {
                    smt_solver: solver,
                    strategy: PdrStrategy::PdrSimplified,
                    ..Default::default()
                },
                Some(tok),
            );
            let _ = tx2.send(("pdr(abc-dc2)".to_string(), res));
        });

        // Worker 3: Craig interpolation (`strash; dc2; int -k`) — excels where PDR stalls
        let m3 = pruned.clone();
        let tx3 = tx.clone();
        let solver3 = cfg.solver.clone();
        let tok3 = Arc::clone(&cancel);
        launched += 1;
        thread::spawn(move || {
            let res = run_pdr_cancellable(
                &m3,
                &PdrConfig {
                    smt_solver: solver3,
                    strategy: PdrStrategy::Interpolation,
                    ..Default::default()
                },
                Some(tok3),
            );
            let _ = tx3.send(("int(abc-dc2)".to_string(), res));
        });
    }

    drop(tx);

    let mut fallback: Option<(String, ProveOutcome)> = None;
    let mut last_err: Option<anyhow::Error> = None;

    for _ in 0..launched {
        let Ok((eng, res)) = rx.recv() else { break };
        match res {
            Ok(out @ ProveOutcome::Proved { k }) => {
                cancel.cancel();
                if let Some(dir) = &cfg.cache_dir {
                    let mut cache = ProofCache::open(dir);
                    let _ = cache.insert(
                        cache_key,
                        CachedVerdict {
                            engine: eng.clone(),
                            k,
                        },
                    );
                }
                return Ok(PortfolioResult {
                    outcome: out,
                    winner: eng,
                    cache_hit: false,
                });
            }
            Ok(out @ ProveOutcome::Fail { .. }) | Ok(out @ ProveOutcome::AssumptionsUnsat { .. }) => {
                cancel.cancel();
                return Ok(PortfolioResult {
                    outcome: out,
                    winner: eng,
                    cache_hit: false,
                });
            }
            Ok(other) => {
                fallback = Some((eng, other));
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    cancel.cancel();

    if let Some((eng, out)) = fallback {
        Ok(PortfolioResult {
            outcome: out,
            winner: eng,
            cache_hit: false,
        })
    } else if let Some(e) = last_err {
        Err(e)
    } else {
        Ok(PortfolioResult {
            outcome: ProveOutcome::Unknown { max_k: cfg.depth },
            winner: "none".into(),
            cache_hit: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CaseSplitSpec {
    pub signal: String,
    pub values: Vec<u64>,
}

/// Parse `--case-split sig=0..3` or `--case-split sig=0,1,2,3`.
pub fn parse_case_split(spec: &str) -> Result<CaseSplitSpec> {
    let (sig, rhs) = spec
        .split_once('=')
        .ok_or_else(|| anyhow!("invalid case-split '{spec}'; expected `signal=0..3` or `signal=0,1,2`"))?;
    let sig = sig.trim().to_string();
    let rhs = rhs.trim();
    let values = if let Some((a, b)) = rhs.split_once("..") {
        let lo: u64 = a.trim().parse()?;
        let hi: u64 = b.trim().parse()?;
        if lo > hi || hi - lo > 64 {
            bail!("case-split range {lo}..{hi} invalid or exceeds 64 cases");
        }
        (lo..=hi).collect()
    } else {
        rhs.split(',')
            .map(|s| s.trim().parse::<u64>().map_err(|e| anyhow!("{e}")))
            .collect::<Result<Vec<_>>>()?
    };
    if values.is_empty() {
        bail!("case-split '{spec}' has no values");
    }
    Ok(CaseSplitSpec {
        signal: sig,
        values,
    })
}

/// Parallel case-splitting proof: partitions the state space across `spec.values`, verifies
/// exhaustiveness (unless covered by existing assumes), and proves every case concurrently via `run_portfolio`.
pub fn run_case_split(
    base: &Module,
    spec: &CaseSplitSpec,
    cfg: &PortfolioConfig,
) -> Result<PortfolioResult> {
    let sid = base
        .find_signal(&spec.signal)
        .ok_or_else(|| anyhow!("case-split signal '{}' not found", spec.signal))?;
    let w = base.signal(sid).width;

    // First verify that the split values are exhaustive (or covered by module assumptions):
    // `assert (sig == v0 || sig == v1 || ...)`!
    let mut exh = base.clone();
    exh.properties.retain(|p| p.kind == PropertyKind::Assume);
    let sig_e = exh.sig_expr(sid);
    let mut any_val = exh.c_bool(false);
    for &v in &spec.values {
        let vc = exh.c_u64(w, v);
        let eq = exh.eq(sig_e, vc, false);
        any_val = exh.logic_or(any_val, eq);
    }
    let one = exh.c_bool(true);
    exh.properties.push(Property {
        name: format!("case_split_exhaustive_{}", spec.signal),
        kind: PropertyKind::Assert,
        cond: any_val,
        en: one,
        source: None,
    });
    let exh_res = run_portfolio(&exh, cfg)?;
    if !matches!(exh_res.outcome, ProveOutcome::Proved { .. }) {
        bail!(
            "case-split on '{}' is not exhaustive over {:?} (side-condition: {:?})",
            spec.signal,
            spec.values,
            exh_res.outcome
        );
    }

    // Launch all split sub-proofs in parallel threads!
    let handles: Vec<_> = spec
        .values
        .iter()
        .map(|&val| {
            let mut sub = base.clone();
            let s_e = sub.sig_expr(sid);
            let vc = sub.c_u64(w, val);
            let eq = sub.eq(s_e, vc, false);
            let one_s = sub.c_bool(true);
            sub.properties.push(Property {
                name: format!("case_split_{}_{val}", spec.signal),
                kind: PropertyKind::Assume,
                cond: eq,
                en: one_s,
                source: None,
            });
            let c = cfg.clone();
            thread::spawn(move || (val, run_portfolio(&sub, &c)))
        })
        .collect();

    let mut winners = Vec::new();
    let mut all_cache = true;
    for h in handles {
        let (val, res) = h
            .join()
            .map_err(|_| anyhow!("case-split worker thread panicked"))?;
        let pr = res?;
        all_cache &= pr.cache_hit;
        match pr.outcome {
            ProveOutcome::Proved { .. } => {
                winners.push(format!("{}={val}:{}", spec.signal, pr.winner));
            }
            fail @ ProveOutcome::Fail { .. } => {
                return Ok(PortfolioResult {
                    outcome: fail,
                    winner: format!("case({}={val})/{}", spec.signal, pr.winner),
                    cache_hit: false,
                });
            }
            other => {
                return Ok(PortfolioResult {
                    outcome: other,
                    winner: format!("case({}={val})/{}", spec.signal, pr.winner),
                    cache_hit: false,
                });
            }
        }
    }

    Ok(PortfolioResult {
        outcome: ProveOutcome::Proved { k: 0 },
        winner: format!("case_split({})", winners.join(",")),
        cache_hit: all_cache,
    })
}
