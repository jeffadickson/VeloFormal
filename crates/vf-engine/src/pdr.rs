use anyhow::{bail, Context, Result};
use std::fs;
use std::process::Command;
use vf_ir::{Module, PropertyKind};
use vf_smt::aiger::bitblast_to_aig;

use crate::bmc::{run_bmc, BmcConfig, BmcOutcome};
use crate::kind::ProveOutcome;

#[derive(Debug, Clone)]
pub struct PdrConfig {
    /// ABC binary (`yosys-abc` or `abc`).
    pub abc_bin: String,
    /// SMT solver used only to reconstruct a word-level VCD counterexample if PDR finds a bug.
    pub smt_solver: String,
}

impl Default for PdrConfig {
    fn default() -> Self {
        Self {
            abc_bin: default_abc_bin(),
            smt_solver: "z3".to_string(),
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

/// Prove unbounded safety via IC3/PDR (`abc pdr` on bit-blasted AIGER 1.0).
/// Unlike k-induction, PDR automatically discovers inductive invariants for non-inductive properties!
pub fn run_pdr(m: &Module, cfg: &PdrConfig) -> Result<ProveOutcome> {
    let has_asserts = m.properties.iter().any(|p| p.kind == PropertyKind::Assert);
    if !has_asserts {
        return Ok(ProveOutcome::Proved { k: 0 });
    }

    let aig = bitblast_to_aig(m)?;
    let bytes = aig.to_binary_aig();

    let tmp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let aig_path = tmp_dir.join(format!("veloformal_pdr_{pid}_{ts}.aig"));
    fs::write(&aig_path, &bytes)
        .with_context(|| format!("failed to write temp AIGER file {}", aig_path.display()))?;

    let abc_cmd = format!("read_aiger {}; pdr", aig_path.display());
    let output = Command::new(&cfg.abc_bin)
        .args(["-c", &abc_cmd])
        .output()
        .with_context(|| format!("failed to run ABC binary '{}'", cfg.abc_bin));
    let _ = fs::remove_file(&aig_path);
    let output = output?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if combined.contains("Property proved.") {
        return Ok(ProveOutcome::Proved { k: 0 });
    }

    if let Some((out_idx, frame)) = parse_abc_cex(&combined) {
        let prop_name = aig
            .bad_outputs
            .get(out_idx)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| format!("assert_{out_idx}"));
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

    if !output.status.success() {
        bail!("ABC PDR failed:\n{combined}");
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
