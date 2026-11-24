use anyhow::{bail, Result};
use std::collections::HashMap;
use vf_ir::{
    AsyncReset, CombAssign, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property, PropertyKind,
    SignalId, SignalKind,
};

use crate::kind::{run_kinduction, ProveConfig, ProveOutcome};

/// Build a miter module comparing all outputs of `gold` and `gate` under shared inputs.
pub fn build_miter(gold: &Module, gate: &Module) -> Result<Module> {
    build_miter_filtered(gold, gate, &[], &[])
}

/// Build a miter module comparing selected outputs of `gold` and `gate` under shared inputs.
/// - If `only_outputs` is non-empty, only those output port names are compared.
/// - Any output port in `ignore_outputs` is skipped (useful for TMR/ECC safety-insertion SLEC).
pub fn build_miter_filtered(
    gold: &Module,
    gate: &Module,
    only_outputs: &[String],
    ignore_outputs: &[String],
) -> Result<Module> {
    let mut miter = Module::new(format!("equiv_{}_vs_{}", gold.name, gate.name));

    // Match inputs and outputs by name & width.
    let mut gold_inputs: HashMap<&str, SignalId> = HashMap::new();
    let mut gold_outputs: HashMap<&str, SignalId> = HashMap::new();
    for s in &gold.signals {
        match s.kind {
            SignalKind::Input => {
                gold_inputs.insert(&s.name, s.id);
            }
            SignalKind::Output => {
                gold_outputs.insert(&s.name, s.id);
            }
            _ => {}
        }
    }

    let mut shared_inputs: HashMap<String, SignalId> = HashMap::new();
    for s in &gold.signals {
        if s.kind == SignalKind::Input {
            let id = miter.add_signal(&s.name, s.width, s.signed, SignalKind::Input);
            miter.signal_mut(id).is_clock = s.is_clock;
            shared_inputs.insert(s.name.clone(), id);
        }
    }

    for s in &gate.signals {
        if s.kind == SignalKind::Input {
            if let Some(&gid) = gold_inputs.get(s.name.as_str()) {
                let gs = gold.signal(gid);
                if gs.width != s.width {
                    bail!(
                        "input port '{}' width mismatch: gold={} vs gate={}",
                        s.name,
                        gs.width,
                        s.width
                    );
                }
            } else {
                let id = miter.add_signal(&s.name, s.width, s.signed, SignalKind::Input);
                shared_inputs.insert(s.name.clone(), id);
            }
        }
    }

    let gold_map = embed_module(&mut miter, gold, "gold", &shared_inputs);
    let gate_map = embed_module(&mut miter, gate, "gate", &shared_inputs);

    let one = miter.c_bool(true);
    let mut compared = 0usize;
    for s in &gate.signals {
        if s.kind == SignalKind::Output {
            if ignore_outputs.iter().any(|ig| ig == &s.name) {
                continue;
            }
            if !only_outputs.is_empty() && !only_outputs.iter().any(|o| o == &s.name) {
                continue;
            }
            let Some(&gid) = gold_outputs.get(s.name.as_str()) else {
                bail!("output port '{}' exists in gate but not in gold", s.name);
            };
            let gs = gold.signal(gid);
            if gs.width != s.width {
                bail!(
                    "output port '{}' width mismatch: gold={} vs gate={}",
                    s.name,
                    gs.width,
                    s.width
                );
            }
            let g_sig = miter.sig_expr(gold_map[&gid]);
            let t_sig = miter.sig_expr(gate_map[&s.id]);
            let eq = miter.eq(g_sig, t_sig, false);
            miter.properties.push(Property {
                name: format!("equiv_{}", s.name),
                kind: PropertyKind::Assert,
                cond: eq,
                en: one,
                source: None,
            });
            compared += 1;
        }
    }

    if compared == 0 {
        bail!("no matching output ports found to check equivalence");
    }

    miter
        .topo_sort_comb()
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(miter)
}

fn embed_module(
    dst: &mut Module,
    src: &Module,
    prefix: &str,
    shared_inputs: &HashMap<String, SignalId>,
) -> HashMap<SignalId, SignalId> {
    let mut sig_map: HashMap<SignalId, SignalId> = HashMap::new();
    for s in &src.signals {
        let mapped = if s.kind == SignalKind::Input {
            shared_inputs[&s.name]
        } else {
            let id = dst.add_signal(
                format!("{prefix}.{}", s.name),
                s.width,
                s.signed,
                if s.kind == SignalKind::Output {
                    SignalKind::Wire
                } else {
                    s.kind
                },
            );
            dst.signal_mut(id).init = s.init.clone();
            id
        };
        sig_map.insert(s.id, mapped);
    }

    let mut mem_map: HashMap<MemId, MemId> = HashMap::new();
    for mem in &src.memories {
        let id = dst.add_memory(
            format!("{prefix}.{}", mem.name),
            mem.addr_width,
            mem.data_width,
            mem.depth,
        );
        dst.memory_mut(id).init = mem.init.clone();
        mem_map.insert(mem.id, id);
    }

    let mut expr_map: Vec<ExprId> = Vec::with_capacity(src.exprs.len());
    for node in &src.exprs {
        let mapped_kind = match &node.kind {
            ExprKind::Const(c) => ExprKind::Const(c.clone()),
            ExprKind::Signal(s) => ExprKind::Signal(sig_map[s]),
            ExprKind::Not(a) => ExprKind::Not(expr_map[a.0]),
            ExprKind::Neg(a) => ExprKind::Neg(expr_map[a.0]),
            ExprKind::RedAnd(a) => ExprKind::RedAnd(expr_map[a.0]),
            ExprKind::RedOr(a) => ExprKind::RedOr(expr_map[a.0]),
            ExprKind::RedXor(a) => ExprKind::RedXor(expr_map[a.0]),
            ExprKind::Add(a, b) => ExprKind::Add(expr_map[a.0], expr_map[b.0]),
            ExprKind::Sub(a, b) => ExprKind::Sub(expr_map[a.0], expr_map[b.0]),
            ExprKind::Mul(a, b) => ExprKind::Mul(expr_map[a.0], expr_map[b.0]),
            ExprKind::UDiv(a, b) => ExprKind::UDiv(expr_map[a.0], expr_map[b.0]),
            ExprKind::SDiv(a, b) => ExprKind::SDiv(expr_map[a.0], expr_map[b.0]),
            ExprKind::UMod(a, b) => ExprKind::UMod(expr_map[a.0], expr_map[b.0]),
            ExprKind::SMod(a, b) => ExprKind::SMod(expr_map[a.0], expr_map[b.0]),
            ExprKind::And(a, b) => ExprKind::And(expr_map[a.0], expr_map[b.0]),
            ExprKind::Or(a, b) => ExprKind::Or(expr_map[a.0], expr_map[b.0]),
            ExprKind::Xor(a, b) => ExprKind::Xor(expr_map[a.0], expr_map[b.0]),
            ExprKind::Shl(a, b) => ExprKind::Shl(expr_map[a.0], expr_map[b.0]),
            ExprKind::Lshr(a, b) => ExprKind::Lshr(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ashr(a, b) => ExprKind::Ashr(expr_map[a.0], expr_map[b.0]),
            ExprKind::Eq(a, b) => ExprKind::Eq(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ne(a, b) => ExprKind::Ne(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ult(a, b) => ExprKind::Ult(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ule(a, b) => ExprKind::Ule(expr_map[a.0], expr_map[b.0]),
            ExprKind::Slt(a, b) => ExprKind::Slt(expr_map[a.0], expr_map[b.0]),
            ExprKind::Sle(a, b) => ExprKind::Sle(expr_map[a.0], expr_map[b.0]),
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => ExprKind::Mux {
                cond: expr_map[cond.0],
                then_val: expr_map[then_val.0],
                else_val: expr_map[else_val.0],
            },
            ExprKind::Concat(parts) => {
                ExprKind::Concat(parts.iter().map(|p| expr_map[p.0]).collect())
            }
            ExprKind::Slice { expr, hi, lo } => ExprKind::Slice {
                expr: expr_map[expr.0],
                hi: *hi,
                lo: *lo,
            },
            ExprKind::Zext { expr, width } => ExprKind::Zext {
                expr: expr_map[expr.0],
                width: *width,
            },
            ExprKind::Sext { expr, width } => ExprKind::Sext {
                expr: expr_map[expr.0],
                width: *width,
            },
            ExprKind::MemRead { mem, addr } => ExprKind::MemRead {
                mem: mem_map[mem],
                addr: expr_map[addr.0],
            },
        };
        let id = dst.intern(node.width, mapped_kind);
        expr_map.push(id);
    }

    for ca in &src.comb {
        dst.comb.push(CombAssign {
            signal: sig_map[&ca.signal],
            value: expr_map[ca.value.0],
        });
    }
    for ff in &src.ffs {
        dst.ffs.push(Ff {
            signal: sig_map[&ff.signal],
            next: expr_map[ff.next.0],
            init: ff.init.clone(),
            clock: ff.clock.map(|c| vf_ir::ClockSpec {
                signal: sig_map[&c.signal],
                edge: c.edge,
            }),
            async_reset: ff.async_reset.as_ref().map(|r| AsyncReset {
                cond: expr_map[r.cond.0],
                reset_val: expr_map[r.reset_val.0],
            }),
        });
    }
    for mw in &src.mem_writes {
        dst.mem_writes.push(MemWrite {
            mem: mem_map[&mw.mem],
            addr: expr_map[mw.addr.0],
            data: expr_map[mw.data.0],
            enable: expr_map[mw.enable.0],
            clock: mw.clock.map(|c| vf_ir::ClockSpec {
                signal: sig_map[&c.signal],
                edge: c.edge,
            }),
        });
    }
    // Preserve assumptions from gold/gate inside the miter.
    for p in &src.properties {
        if p.kind == PropertyKind::Assume {
            dst.properties.push(Property {
                name: format!("{prefix}.{}", p.name),
                kind: PropertyKind::Assume,
                cond: expr_map[p.cond.0],
                en: expr_map[p.en.0],
                source: p.source.clone(),
            });
        }
    }

    sig_map
}

/// Questa SLEC (Sequential Logic Equivalence Checking) configuration supporting
/// pipeline latency offsets, reset sequences, safety-insertion output filtering,
/// Low-Power Clock Gating (LPGC) observability qualifiers, and k-induction / ABC PDR backends.
#[derive(Debug, Clone)]
pub struct SlecConfig {
    pub depth: usize,
    pub solver: String,
    /// `"auto"`, `"kind"`, or `"pdr"`.
    pub engine: String,
    /// Extra delay cycles to apply to `gold` outputs (use when `gate` has more pipeline stages).
    pub latency_gold: usize,
    /// Extra delay cycles to apply to `gate` outputs (use when `gold` has more pipeline stages).
    pub latency_gate: usize,
    /// Number of initial cycles to mask comparison while pipeline/reset settles.
    pub skip_cycles: usize,
    /// Optional reset initialization sequence `(signal_name, active_value)` held for `reset_cycles`.
    pub reset: Option<(String, u64)>,
    /// Number of initial cycles to assert `reset` before deasserting (defaults to 1 when `reset` is set).
    pub reset_cycles: usize,
    /// Optional subset of output ports to compare (empty = all outputs).
    pub outputs: Vec<String>,
    /// Output ports to exclude from equivalence comparison (e.g. extra `err_flag` in TMR/ECC gate RTL).
    pub ignore_outputs: Vec<String>,
    /// Optional Low-Power Clock Gating (LPGC) / observability condition (compare outputs only when true).
    pub valid_cond: Option<String>,
}

impl Default for SlecConfig {
    fn default() -> Self {
        Self {
            depth: 20,
            solver: "z3".to_string(),
            engine: "auto".to_string(),
            latency_gold: 0,
            latency_gate: 0,
            skip_cycles: 0,
            reset: None,
            reset_cycles: 1,
            outputs: Vec::new(),
            ignore_outputs: Vec::new(),
            valid_cond: None,
        }
    }
}

/// Parse a CLI `--reset` specification like `"rst=1"` or `"rst_n=0"`.
pub fn parse_reset_spec(spec: &str) -> Result<(String, u64)> {
    if let Some((name, val)) = spec.split_once('=') {
        let v: u64 = val
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid reset value in '{spec}': {e}"))?;
        Ok((name.trim().to_string(), v))
    } else {
        let name = spec.trim();
        let active = if name.to_ascii_lowercase().ends_with("_n")
            || name.to_ascii_lowercase().ends_with("resetn")
        {
            0
        } else {
            1
        };
        Ok((name.to_string(), active))
    }
}

/// Build an SLEC miter with optional pipeline latency alignment, reset sequence,
/// output filtering, and LPGC observability qualifier.
pub fn build_slec_miter(gold: &Module, gate: &Module, cfg: &SlecConfig) -> Result<Module> {
    let mut miter = build_miter_filtered(gold, gate, &cfg.outputs, &cfg.ignore_outputs)?;
    let max_lat = cfg.latency_gold.max(cfg.latency_gate);
    let rst_cycles = if cfg.reset.is_some() {
        cfg.reset_cycles.max(1)
    } else {
        0
    };
    let settle = cfg.skip_cycles.max(rst_cycles + max_lat);

    // 1. Optional Reset-Sequence Initialization (`Assume(in_reset => rst == active_val)`).
    if let Some((rst_name, active_val)) = &cfg.reset {
        let rst_sid = miter
            .find_signal(rst_name)
            .ok_or_else(|| anyhow::anyhow!("SLEC reset signal '{rst_name}' not found"))?;
        let rst_w = miter.signal(rst_sid).width;

        // Build `$slec_in_reset` shift register: 1 for cycles `0..rst_cycles`, 0 forever after.
        let mut chain = miter.c_bool(false);
        for i in 0..rst_cycles {
            let s = miter.add_signal(format!("$slec_in_rst_{i}"), 1, false, SignalKind::Reg);
            miter.signal_mut(s).init = Some(vf_ir::ConstBits::one(1));
            miter.ffs.push(Ff {
                signal: s,
                next: chain,
                init: Some(vf_ir::ConstBits::one(1)),
                clock: None,
                async_reset: None,
            });
            chain = miter.sig_expr(s);
        }

        let rst_e = miter.sig_expr(rst_sid);
        let active_c = miter.c_u64(rst_w, *active_val);
        let is_active = miter.eq(rst_e, active_c, false);
        miter.properties.push(Property {
            name: "$slec_reset_seq".into(),
            kind: PropertyKind::Assume,
            cond: is_active,
            en: chain,
            source: None,
        });
    }

    // 2. Optional settling enable: 0 for cycles `0..settle`, then 1 forever via shift chain.
    let mut valid_en = miter.c_bool(true);
    for i in 0..settle {
        let s = miter.add_signal(format!("$slec_valid_{i}"), 1, false, SignalKind::Reg);
        miter.signal_mut(s).init = Some(vf_ir::ConstBits::zero(1));
        miter.ffs.push(Ff {
            signal: s,
            next: valid_en,
            init: Some(vf_ir::ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });
        valid_en = miter.sig_expr(s);
    }

    // 3. Optional Low-Power Clock Gating / observability qualifier (`--valid-cond`).
    if let Some(cond_str) = &cfg.valid_cond {
        let qual = parse_slec_cond(&mut miter, cond_str)?;
        valid_en = miter.logic_and(valid_en, qual);
    }

    if cfg.latency_gold == 0 && cfg.latency_gate == 0 && settle == 0 && cfg.valid_cond.is_none() {
        return Ok(miter);
    }

    // 4. Replace equiv properties with latency-aligned comparisons gated by `valid_en`.
    let old_props = std::mem::take(&mut miter.properties);
    let mut delay_counter = 0usize;
    for mut p in old_props {
        if p.kind == PropertyKind::Assert {
            if let ExprKind::Eq(mut a, mut b) = miter.expr(p.cond).kind.clone() {
                for _ in 0..cfg.latency_gold {
                    let w = miter.width(a);
                    let dsig = miter.add_signal(
                        format!("$slec_gold_d_{delay_counter}"),
                        w,
                        false,
                        SignalKind::Reg,
                    );
                    delay_counter += 1;
                    miter.ffs.push(Ff {
                        signal: dsig,
                        next: a,
                        init: None,
                        clock: None,
                        async_reset: None,
                    });
                    a = miter.sig_expr(dsig);
                }
                for _ in 0..cfg.latency_gate {
                    let w = miter.width(b);
                    let dsig = miter.add_signal(
                        format!("$slec_gate_d_{delay_counter}"),
                        w,
                        false,
                        SignalKind::Reg,
                    );
                    delay_counter += 1;
                    miter.ffs.push(Ff {
                        signal: dsig,
                        next: b,
                        init: None,
                        clock: None,
                        async_reset: None,
                    });
                    b = miter.sig_expr(dsig);
                }
                p.cond = miter.eq(a, b, false);
            }
            p.en = miter.logic_and(p.en, valid_en);
        }
        miter.properties.push(p);
    }

    Ok(miter)
}

fn parse_slec_cond(m: &mut Module, expr: &str) -> Result<ExprId> {
    let s = expr.trim();
    if let Some((a, b)) = s.split_once("&&") {
        let la = parse_slec_cond(m, a)?;
        let lb = parse_slec_cond(m, b)?;
        return Ok(m.logic_and(la, lb));
    }
    if let Some((a, b)) = s.split_once("||") {
        let la = parse_slec_cond(m, a)?;
        let lb = parse_slec_cond(m, b)?;
        return Ok(m.logic_or(la, lb));
    }
    if let Some((lhs, rhs)) = s.split_once("==") {
        let l = resolve_slec_atom(m, lhs.trim(), None)?;
        let w = m.width(l);
        let r = resolve_slec_atom(m, rhs.trim(), Some(w))?;
        return Ok(m.eq(l, r, false));
    }
    if let Some((lhs, rhs)) = s.split_once("!=") {
        let l = resolve_slec_atom(m, lhs.trim(), None)?;
        let w = m.width(l);
        let r = resolve_slec_atom(m, rhs.trim(), Some(w))?;
        let eq = m.eq(l, r, false);
        return Ok(m.not(eq));
    }
    if let Some(rest) = s.strip_prefix('!') {
        let inner = parse_slec_cond(m, rest)?;
        return Ok(m.logic_not(inner));
    }
    let atom = resolve_slec_atom(m, s, None)?;
    Ok(m.to_bool(atom))
}

fn resolve_slec_atom(m: &mut Module, tok: &str, width_hint: Option<u32>) -> Result<ExprId> {
    if let Some(sid) = m
        .find_signal(tok)
        .or_else(|| m.find_signal(&format!("gold.{tok}")))
        .or_else(|| m.find_signal(&format!("gate.{tok}")))
    {
        return Ok(m.sig_expr(sid));
    }
    let parsed = if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = tok.strip_prefix("0b").or_else(|| tok.strip_prefix("0B")) {
        u64::from_str_radix(bin, 2).ok()
    } else {
        tok.parse::<u64>().ok()
    };
    if let Some(v) = parsed {
        return Ok(m.c_u64(width_hint.unwrap_or(32), v));
    }
    bail!("unknown signal or literal '{tok}' in SLEC --valid-cond")
}

pub fn check_slec(gold: &Module, gate: &Module, cfg: &SlecConfig) -> Result<(Module, ProveOutcome)> {
    let miter = build_slec_miter(gold, gate, cfg)?;
    let outcome = match cfg.engine.as_str() {
        "pdr" => crate::pdr::run_pdr(
            &miter,
            &crate::pdr::PdrConfig {
                smt_solver: cfg.solver.clone(),
                ..Default::default()
            },
        )?,
        "kind" => run_kinduction(
            &miter,
            &ProveConfig {
                max_k: cfg.depth,
                solver: cfg.solver.clone(),
            },
        )?,
        _ => {
            let k = run_kinduction(
                &miter,
                &ProveConfig {
                    max_k: cfg.depth.min(8),
                    solver: cfg.solver.clone(),
                },
            )?;
            match k {
                ProveOutcome::Unknown { .. } => crate::pdr::run_pdr(
                    &miter,
                    &crate::pdr::PdrConfig {
                        smt_solver: cfg.solver.clone(),
                        ..Default::default()
                    },
                )
                .unwrap_or(k),
                other => other,
            }
        }
    };
    Ok((miter, outcome))
}

pub fn check_equiv(gold: &Module, gate: &Module, cfg: &ProveConfig) -> Result<(Module, ProveOutcome)> {
    let miter = build_miter(gold, gate)?;
    let outcome = run_kinduction(&miter, cfg)?;
    Ok((miter, outcome))
}
