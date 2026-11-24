use anyhow::{anyhow, bail, Result};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use vf_ir::{Module, Property, PropertyKind};

use crate::cache::{module_fingerprint, CachedVerdict, ProofCache};
use crate::kind::{run_kinduction, ProveConfig, ProveOutcome};
use crate::pdr::{run_pdr, PdrConfig};

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

/// True parallel multi-engine portfolio orchestrator:
/// Races k-induction (Z3, plus Bitwuzla if installed) and ABC IC3/PDR concurrently on worker threads
/// and returns immediately on the first decisive verdict (`Proved`, `Fail`, or `AssumptionsUnsat`).
/// Supports optional content-addressed COI proof caching.
pub fn run_portfolio(module: &Module, cfg: &PortfolioConfig) -> Result<PortfolioResult> {
    let mut pruned = module.clone();
    vf_ir::passes::coi_prune(&mut pruned);

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

    // Worker 1: k-induction with configured SMT solver (default Z3)
    {
        let m = pruned.clone();
        let tx1 = tx.clone();
        let depth = cfg.depth;
        let solver = cfg.solver.clone();
        launched += 1;
        thread::spawn(move || {
            let res = run_kinduction(
                &m,
                &ProveConfig {
                    max_k: depth,
                    solver: solver.clone(),
                },
            );
            let _ = tx1.send((format!("kind({solver})"), res));
        });
    }

    // Worker 2: IC3/PDR (yosys-abc pdr) concurrently!
    if cfg.enable_pdr {
        let m = pruned.clone();
        let tx2 = tx.clone();
        let solver = cfg.solver.clone();
        launched += 1;
        thread::spawn(move || {
            let res = run_pdr(
                &m,
                &PdrConfig {
                    smt_solver: solver,
                    ..Default::default()
                },
            );
            let _ = tx2.send(("pdr(abc)".to_string(), res));
        });
    }

    drop(tx);

    let mut fallback: Option<(String, ProveOutcome)> = None;
    let mut last_err: Option<anyhow::Error> = None;

    for _ in 0..launched {
        let Ok((eng, res)) = rx.recv() else { break };
        match res {
            Ok(out @ ProveOutcome::Proved { k }) => {
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
