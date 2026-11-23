use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{ConstBits, ExprId, ExprKind, Ff, Module, Property, PropertyKind, SignalKind};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct ConnRule {
    pub name: String,
    pub src: String,
    pub dst: String,
    /// Optional condition: e.g. `sel == 1`, `en`, `!rst`, or empty for unconditional.
    pub cond: Option<String>,
    pub invert: bool,
    pub delay: usize,
}

#[derive(Debug, Clone)]
pub enum ConnStatus {
    Proved,
    Fail { step: usize, trace: Vec<TraceFrame> },
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ConnResult {
    pub rule: ConnRule,
    pub status: ConnStatus,
}

/// Parse CSV connectivity specification:
/// `src,dst[,cond[,invert[,delay]]]` (lines starting with `#` or blank lines ignored).
pub fn parse_connectivity_csv(path: &Path) -> Result<Vec<ConnRule>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read connectivity spec {}", path.display()))?;
    parse_connectivity_str(&text)
}

pub fn parse_connectivity_str(text: &str) -> Result<Vec<ConnRule>> {
    let mut rules = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(|c| c.trim()).collect();
        if cols.len() < 2 {
            bail!("connectivity CSV line {}: expected at least `src,dst`", idx + 1);
        }
        if rules.is_empty()
            && cols[0].eq_ignore_ascii_case("src")
            && cols[1].eq_ignore_ascii_case("dst")
        {
            continue;
        }
        let src = cols[0].to_string();
        let dst = cols[1].to_string();
        let cond = cols
            .get(2)
            .copied()
            .filter(|s| !s.is_empty() && *s != "1" && !s.eq_ignore_ascii_case("true"))
            .map(String::from);
        let invert = cols
            .get(3)
            .map(|s| matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "inv"))
            .unwrap_or(false);
        let delay: usize = cols
            .get(4)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().context("invalid connectivity delay"))
            .transpose()?
            .unwrap_or(0);
        rules.push(ConnRule {
            name: format!("conn_{src}_to_{dst}_{}", rules.len()),
            src,
            dst,
            cond,
            invert,
            delay,
        });
    }
    Ok(rules)
}

/// Questa Connectivity Check equivalent: exhaustively verifies static and conditional
/// pin-to-pin / bus routing rules (with optional polarity inversion and pipeline delay).
pub fn run_connectivity(
    base_module: &Module,
    rules: &[ConnRule],
    depth: usize,
    solver: &str,
) -> Result<Vec<ConnResult>> {
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let mut m = base_module.clone();
        m.properties.retain(|p| p.kind == PropertyKind::Assume);

        let src_sig = m
            .find_signal(&rule.src)
            .ok_or_else(|| anyhow!("connectivity source signal '{}' not found", rule.src))?;
        let dst_sig = m
            .find_signal(&rule.dst)
            .ok_or_else(|| anyhow!("connectivity destination signal '{}' not found", rule.dst))?;

        let mut src_e = m.sig_expr(src_sig);
        if rule.invert {
            src_e = m.not(src_e);
        }
        let dst_e = m.sig_expr(dst_sig);

        let mut en_e = match &rule.cond {
            Some(c) => parse_simple_cond(&mut m, c)?,
            None => m.c_bool(true),
        };

        // Apply pipeline delay if `rule.delay > 0`: compare `past(src, delay)` to `dst` when `past(en, delay)` held.
        for d in 0..rule.delay {
            let sw = m.width(src_e);
            let sreg = m.add_signal(format!("$conn_src_d{d}"), sw, false, SignalKind::Reg);
            m.ffs.push(Ff {
                signal: sreg,
                next: src_e,
                init: None,
                clock: None,
                async_reset: None,
            });
            src_e = m.sig_expr(sreg);

            let ereg = m.add_signal(format!("$conn_en_d{d}"), 1, false, SignalKind::Reg);
            m.signal_mut(ereg).init = Some(ConstBits::zero(1));
            m.ffs.push(Ff {
                signal: ereg,
                next: en_e,
                init: Some(ConstBits::zero(1)),
                clock: None,
                async_reset: None,
            });
            en_e = m.sig_expr(ereg);
        }

        let eq = m.eq(src_e, dst_e, false);
        m.properties.push(Property {
            name: rule.name.clone(),
            kind: PropertyKind::Assert,
            cond: eq,
            en: en_e,
            source: None,
        });
        vf_ir::passes::coi_prune(&mut m);

        let status = match run_kinduction(
            &m,
            &ProveConfig {
                max_k: depth.min(8),
                solver: solver.to_string(),
            },
        )? {
            ProveOutcome::Proved { .. } => ConnStatus::Proved,
            ProveOutcome::Fail { step, trace, .. } => ConnStatus::Fail { step, trace },
            ProveOutcome::Unknown { .. } => match run_pdr(
                &m,
                &PdrConfig {
                    smt_solver: solver.to_string(),
                    ..Default::default()
                },
            ) {
                Ok(ProveOutcome::Proved { .. }) => ConnStatus::Proved,
                Ok(ProveOutcome::Fail { step, trace, .. }) => ConnStatus::Fail { step, trace },
                _ => ConnStatus::Unknown,
            },
            ProveOutcome::AssumptionsUnsat { .. } => ConnStatus::Unknown,
        };
        out.push(ConnResult {
            rule: rule.clone(),
            status,
        });
    }
    Ok(out)
}

/// Parse lightweight condition expressions like `sel == 2`, `en && !rst`, `mode != 0`, `!stall`.
pub fn parse_simple_cond(m: &mut Module, expr: &str) -> Result<ExprId> {
    let s = expr.trim();
    if let Some((a, b)) = s.split_once("&&") {
        let la = parse_simple_cond(m, a)?;
        let lb = parse_simple_cond(m, b)?;
        return Ok(m.logic_and(la, lb));
    }
    if let Some((a, b)) = s.split_once("||") {
        let la = parse_simple_cond(m, a)?;
        let lb = parse_simple_cond(m, b)?;
        return Ok(m.logic_or(la, lb));
    }
    if let Some((lhs, rhs)) = s.split_once("==") {
        let l = parse_atom(m, lhs.trim())?;
        let w = m.width(l);
        let r = parse_atom_with_width(m, rhs.trim(), w)?;
        return Ok(m.eq(l, r, false));
    }
    if let Some((lhs, rhs)) = s.split_once("!=") {
        let l = parse_atom(m, lhs.trim())?;
        let w = m.width(l);
        let r = parse_atom_with_width(m, rhs.trim(), w)?;
        let eq = m.eq(l, r, false);
        return Ok(m.not(eq));
    }
    if let Some(rest) = s.strip_prefix('!') {
        let inner = parse_simple_cond(m, rest)?;
        return Ok(m.logic_not(inner));
    }
    let atom = parse_atom(m, s)?;
    Ok(m.to_bool(atom))
}

fn parse_atom(m: &mut Module, tok: &str) -> Result<ExprId> {
    if let Some(sig) = m.find_signal(tok) {
        return Ok(m.sig_expr(sig));
    }
    if let Some(v) = parse_int(tok) {
        return Ok(m.c_u64(32, v));
    }
    bail!("unknown signal or literal '{tok}' in condition")
}

fn parse_atom_with_width(m: &mut Module, tok: &str, width: u32) -> Result<ExprId> {
    if let Some(sig) = m.find_signal(tok) {
        return Ok(m.sig_expr(sig));
    }
    if let Some(v) = parse_int(tok) {
        return Ok(m.c_u64(width, v));
    }
    bail!("unknown signal or literal '{tok}' in condition")
}

fn parse_int(tok: &str) -> Option<u64> {
    if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = tok.strip_prefix("0b").or_else(|| tok.strip_prefix("0B")) {
        u64::from_str_radix(bin, 2).ok()
    } else {
        tok.parse::<u64>().ok()
    }
}

#[allow(dead_code)]
fn _unused_expr_kind(_: ExprKind) {}
