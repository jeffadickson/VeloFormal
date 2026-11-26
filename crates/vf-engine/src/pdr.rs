use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use vf_ir::{Module, PropertyKind};
use vf_smt::aiger::bitblast_to_aig;
use vf_smt::solver::CancelToken;

use crate::bmc::{run_bmc, BmcConfig, BmcOutcome};
use crate::kind::ProveOutcome;

static AIG_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdrStrategy {
    /// Combinational + sequential AIG simplification (`strash; dc2; scorr -C 500; dc2`) followed by IC3/PDR.
    PdrSimplified,
    /// Raw ABC `pdr` with no pre-simplification.
    PdrRaw,
    /// Combinational AIG simplification (`strash; dc2`) followed by Craig interpolation (`int -k`).
    Interpolation,
}

impl Default for PdrStrategy {
    fn default() -> Self {
        Self::PdrSimplified
    }
}

#[derive(Debug, Clone)]
pub struct PdrConfig {
    /// ABC binary (`yosys-abc` or `abc`).
    pub abc_bin: String,
    /// SMT solver used only to reconstruct a word-level VCD counterexample if ABC finds a bug.
    pub smt_solver: String,
    /// Bit-level verification strategy (AIG pre-simplification + PDR or Craig interpolation).
    pub strategy: PdrStrategy,
}

impl Default for PdrConfig {
    fn default() -> Self {
        Self {
            abc_bin: default_abc_bin(),
            smt_solver: "z3".to_string(),
            strategy: PdrStrategy::PdrSimplified,
        }
    }
}

fn default_abc_bin() -> String {
    for cand in ["yosys-abc", "abc"] {
        if Command::new(cand)
            .args(["-c", "version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return cand.to_string();
        }
    }
    "yosys-abc".to_string()
}

/// Prove unbounded safety via ABC bit-level engines (`strash; dc2; scorr; dc2; pdr` or `int -k`)
/// on bit-blasted AIGER 1.0. Unlike k-induction, PDR & Craig interpolation automatically discover
/// inductive invariants for non-inductive properties.
pub fn run_pdr(m: &Module, cfg: &PdrConfig) -> Result<ProveOutcome> {
    run_pdr_cancellable(m, cfg, None)
}

/// Cancellable ABC bit-level engine runner used by the parallel portfolio.
pub fn run_pdr_cancellable(
    m: &Module,
    cfg: &PdrConfig,
    cancel: Option<Arc<CancelToken>>,
) -> Result<ProveOutcome> {
    let has_asserts = m.properties.iter().any(|p| p.kind == PropertyKind::Assert);
    if !has_asserts {
        return Ok(ProveOutcome::Proved { k: 0 });
    }
    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let aig = bitblast_to_aig(m)?;
    if cfg.strategy == PdrStrategy::Interpolation && aig.latches.is_empty() {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }
    let bytes = aig.to_binary_aig();

    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let tmp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let seq = AIG_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let aig_path = tmp_dir.join(format!("veloformal_abc_{pid}_{ts}_{seq}.aig"));
    fs::write(&aig_path, &bytes)
        .with_context(|| format!("failed to write temp AIGER file {}", aig_path.display()))?;

    let engine_script = match cfg.strategy {
        PdrStrategy::PdrSimplified => "strash; dc2; scorr -C 500; dc2; pdr",
        PdrStrategy::PdrRaw => "pdr",
        PdrStrategy::Interpolation => "strash; dc2; int -k",
    };
    let abc_cmd = format!("read_aiger {}; {engine_script}", aig_path.display());

    let mut child = match Command::new(&cfg.abc_bin)
        .args(["-c", &abc_cmd])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&aig_path);
            return Err(e).with_context(|| format!("failed to run ABC binary '{}'", cfg.abc_bin));
        }
    };

    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();
    let child_slot = match &cancel {
        Some(tok) => tok.register(child),
        None => Arc::new(std::sync::Mutex::new(Some(child))),
    };

    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    std::thread::scope(|s| {
        let err_handle = s.spawn(|| {
            let mut sbuf = String::new();
            if let Some(mut err) = child_stderr.take() {
                let _ = err.read_to_string(&mut sbuf);
            }
            sbuf
        });
        if let Some(mut out) = child_stdout.take() {
            let _ = out.read_to_string(&mut stdout_buf);
        }
        stderr_buf = err_handle.join().unwrap_or_default();
    });

    let status = if let Ok(mut guard) = child_slot.lock() {
        guard.take().and_then(|mut c| c.wait().ok())
    } else {
        None
    };
    let _ = fs::remove_file(&aig_path);

    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let combined = format!("{stdout_buf}\n{stderr_buf}");

    if combined.contains("Property proved.") {
        return Ok(ProveOutcome::Proved { k: 0 });
    }

    if let Some((out_idx, frame)) = parse_abc_cex(&combined) {
        let prop_name = aig
            .bad_outputs
            .get(out_idx)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| format!("assert_{out_idx}"));
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Ok(ProveOutcome::Unknown { max_k: 0 });
        }
        // Reconstruct a clean word-level trace via BMC up to exact frame + 1.
        let bmc = run_bmc(
            m,
            &BmcConfig {
                depth: frame + 1,
                solver: cfg.smt_solver.clone(),
                skip: 0,
                append: 0,
            },
        )?;
        match bmc {
            BmcOutcome::Fail {
                property,
                step,
                trace,
            } => {
                return Ok(ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                });
            }
            _ => {
                return Ok(ProveOutcome::Fail {
                    property: prop_name,
                    step: frame,
                    trace: Vec::new(),
                });
            }
        }
    }

    if combined.contains("Does not work for combinational networks") {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    if let Some(st) = status {
        if !st.success() {
            bail!("ABC ({engine_script}) failed:\n{combined}");
        }
    }

    Ok(ProveOutcome::Unknown { max_k: 0 })
}

fn parse_abc_cex(text: &str) -> Option<(usize, usize)> {
    // ABC prints: `Output 0 of miter "..." was asserted in frame 5.  Time = 0.02 sec`
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Output ") {
            let mut parts = rest.split_whitespace();
            let out_idx: usize = parts.next()?.parse().ok()?;
            if let Some(pos) = line.find("in frame ") {
                let frame_tok = line[pos + "in frame ".len()..]
                    .split_whitespace()
                    .next()?
                    .trim_end_matches('.');
                let frame: usize = frame_tok.parse().ok()?;
                return Some((out_idx, frame));
            }
        }
    }
    None
}
