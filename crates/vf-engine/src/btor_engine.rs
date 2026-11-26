use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use vf_ir::{Module, PropertyKind};
use vf_smt::btor2::export_btor2;
use vf_smt::solver::CancelToken;

use crate::bmc::{run_bmc, BmcConfig, BmcOutcome};
use crate::kind::ProveOutcome;

static BTOR_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtorSolver {
    /// Auto-select best installed word-level BTOR2 solver (`pono` -> `avr` -> `btormc`).
    Auto,
    /// Pono word-level IC3 with Implicit Predicate Abstraction (`pono -e ic3ia`).
    PonoIc3ia,
    /// Pono word-level k-induction (`pono -e ind`).
    PonoKind,
    /// AVR word-level IC3/PDR (`avr`).
    Avr,
    /// Boolector/Btor2Tools bounded model checker (`btormc`).
    Btormc,
}

impl Default for BtorSolver {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone)]
pub struct BtorConfig {
    pub solver: BtorSolver,
    pub depth: usize,
    /// SMT solver used to reconstruct a clean word-level VCD trace when BTOR2 finds a counterexample.
    pub smt_solver: String,
}

impl Default for BtorConfig {
    fn default() -> Self {
        Self {
            solver: BtorSolver::Auto,
            depth: 20,
            smt_solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Btor2Summary {
    pub nodes: usize,
    pub states: usize,
    pub inputs: usize,
    pub bads: usize,
    pub constraints: usize,
    pub arrays: usize,
}

/// Inspect and validate the BTOR2 serialization of `m` (verifies structural well-formedness).
pub fn validate_btor2_export(m: &Module) -> Result<(String, Btor2Summary)> {
    let text = export_btor2(m);
    let mut sum = Btor2Summary {
        nodes: 0,
        states: 0,
        inputs: 0,
        bads: 0,
        constraints: 0,
        arrays: 0,
    };
    let mut max_id = 0usize;
    for (ln, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let id: usize = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("BTOR2 line {}: invalid node id", ln + 1))?;
        if id <= max_id {
            bail!("BTOR2 line {}: non-monotonic node id {id} <= {max_id}", ln + 1);
        }
        max_id = id;
        let op = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("BTOR2 line {}: missing opcode", ln + 1))?;
        sum.nodes += 1;
        match op {
            "state" => sum.states += 1,
            "input" => sum.inputs += 1,
            "bad" => sum.bads += 1,
            "constraint" => sum.constraints += 1,
            "sort" if parts.next() == Some("array") => sum.arrays += 1,
            _ => {}
        }
    }
    Ok((text, sum))
}

/// Detect the best installed word-level BTOR2 model checker on `PATH`.
pub fn detect_btor_solver() -> Option<(BtorSolver, &'static str)> {
    for (kind, bin, flag) in [
        (BtorSolver::PonoIc3ia, "pono", "--version"),
        (BtorSolver::Avr, "avr", "--help"),
        (BtorSolver::Btormc, "btormc", "-h"),
    ] {
        if Command::new(bin)
            .arg(flag)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some((kind, bin));
        }
    }
    None
}

/// Run a word-level BTOR2 model checker (`pono`, `avr`, or `btormc`) directly on `vf_ir::Module`
/// without bit-blasting wide datapaths or arrays.
pub fn run_btor(m: &Module, cfg: &BtorConfig) -> Result<ProveOutcome> {
    run_btor_cancellable(m, cfg, None)
}

/// Cancellable word-level BTOR2 runner compatible with VeloFormal's parallel portfolio `CancelToken`.
pub fn run_btor_cancellable(
    m: &Module,
    cfg: &BtorConfig,
    cancel: Option<Arc<CancelToken>>,
) -> Result<ProveOutcome> {
    let has_asserts = m.properties.iter().any(|p| p.kind == PropertyKind::Assert);
    if !has_asserts {
        return Ok(ProveOutcome::Proved { k: 0 });
    }
    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let (solver_kind, bin) = match cfg.solver {
        BtorSolver::Auto => match detect_btor_solver() {
            Some(found) => found,
            None => bail!(
                "no word-level BTOR2 solver found on PATH (install `pono`, `avr`, or `btormc`)"
            ),
        },
        BtorSolver::PonoIc3ia => (BtorSolver::PonoIc3ia, "pono"),
        BtorSolver::PonoKind => (BtorSolver::PonoKind, "pono"),
        BtorSolver::Avr => (BtorSolver::Avr, "avr"),
        BtorSolver::Btormc => (BtorSolver::Btormc, "btormc"),
    };

    let (btor_text, _) = validate_btor2_export(m)?;
    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let tmp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let seq = BTOR_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let btor_path = tmp_dir.join(format!("veloformal_btor_{pid}_{ts}_{seq}.btor2"));
    fs::write(&btor_path, &btor_text)
        .with_context(|| format!("failed to write temp BTOR2 file {}", btor_path.display()))?;

    let mut cmd = Command::new(bin);
    match solver_kind {
        BtorSolver::PonoIc3ia | BtorSolver::Auto => {
            cmd.args(["-e", "ic3ia", "-k", &cfg.depth.to_string()]);
            cmd.arg(&btor_path);
        }
        BtorSolver::PonoKind => {
            cmd.args(["-e", "ind", "-k", &cfg.depth.to_string()]);
            cmd.arg(&btor_path);
        }
        BtorSolver::Avr => {
            cmd.arg(&btor_path);
        }
        BtorSolver::Btormc => {
            cmd.args(["-kmax", &cfg.depth.to_string()]);
            cmd.arg(&btor_path);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&btor_path);
            return Err(e).with_context(|| format!("failed to spawn BTOR2 solver '{bin}'"));
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

    let _ = if let Ok(mut guard) = child_slot.lock() {
        guard.take().and_then(|mut c| c.wait().ok())
    } else {
        None
    };
    let _ = fs::remove_file(&btor_path);

    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Ok(ProveOutcome::Unknown { max_k: 0 });
    }

    let combined = format!("{stdout_buf}\n{stderr_buf}");
    let lower = combined.to_ascii_lowercase();

    // Pono / AVR / BTOR2 standard verdicts:
    // - `unsat` / `proved` / `h` (AVR) for unbounded engines (`pono`, `avr`) -> Proved
    // - `sat` / `falsified` / `b0` witness -> Fail (reconstruct word-level trace via BMC)
    let is_unbounded_engine = !matches!(solver_kind, BtorSolver::Btormc);
    if is_unbounded_engine
        && (lower.lines().any(|l| l.trim() == "unsat")
            || lower.contains("property holds")
            || lower.contains("verification successful"))
    {
        return Ok(ProveOutcome::Proved { k: 0 });
    }

    if lower.lines().any(|l| l.trim() == "sat" || l.trim().starts_with("b0"))
        || lower.contains("falsified")
    {
        let bmc_depth = parse_btor_witness_depth(&combined).unwrap_or(cfg.depth) + 1;
        if let Ok(BmcOutcome::Fail {
            property,
            step,
            trace,
        }) = run_bmc(
            m,
            &BmcConfig {
                depth: bmc_depth,
                solver: cfg.smt_solver.clone(),
                skip: 0,
                append: 0,
            },
        ) {
            return Ok(ProveOutcome::Fail {
                property,
                step,
                trace,
            });
        }
        let prop = m
            .properties
            .iter()
            .find(|p| p.kind == PropertyKind::Assert)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "assert_0".into());
        return Ok(ProveOutcome::Fail {
            property: prop,
            step: bmc_depth.saturating_sub(1),
            trace: Vec::new(),
        });
    }

    Ok(ProveOutcome::Unknown { max_k: cfg.depth })
}

fn parse_btor_witness_depth(text: &str) -> Option<usize> {
    // BTOR2 witness frames use `@0`, `@1`, ..., `@k` markers.
    let mut max_frame: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('@') {
            if let Ok(n) = rest.parse::<usize>() {
                max_frame = Some(max_frame.map_or(n, |cur| cur.max(n)));
            }
        }
    }
    max_frame
}
