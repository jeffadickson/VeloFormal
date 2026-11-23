use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{
    AsyncReset, CombAssign, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property, PropertyKind,
    SignalId, SignalKind,
};
use vf_smt::solver::TraceFrame;

use crate::connectivity::parse_simple_cond;

#[derive(Debug, Clone)]
pub struct SecureConfig {
    /// Source signal(s) whose values may differ between the two self-composed executions
    /// (secret assets for confidentiality, or untrusted inputs for integrity).
    pub sources: Vec<String>,
    /// Target signal(s) that must remain identical across both executions
    /// (public outputs for confidentiality, or protected assets for integrity).
    pub targets: Vec<String>,
    /// Optional condition under which information flow is allowed (declassification / authorized write).
    pub declassify: Option<String>,
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

#[derive(Debug, Clone)]
pub enum SecureOutcome {
    /// Strictly non-interfering: NO information flow from `sources` to `targets` for all time!
    Secure,
    /// Concrete 2-execution counterexample demonstrating an information leak / integrity violation!
    Leak { step: usize, trace: Vec<TraceFrame> },
    Unknown,
}

/// Questa Secure Check equivalent: formal Information Flow Tracking (confidentiality & integrity)
/// using 2-copy self-composition (`copy_a` + `copy_b`).
///
/// All inputs EXCEPT `sources` are tied equal across copies A and B.
/// If `targets` can ever differ between A and B (outside any `declassify` guard),
/// there is a provable information flow from `sources` to `targets`.
pub fn run_secure_check(base: &Module, cfg: &SecureConfig) -> Result<(Module, SecureOutcome)> {
    if cfg.sources.is_empty() || cfg.targets.is_empty() {
        bail!("secure check requires at least one source signal and one target signal");
    }

    let source_ids: HashSet<SignalId> = cfg
        .sources
        .iter()
        .map(|name| {
            base.find_signal(name)
                .ok_or_else(|| anyhow!("secure source signal '{name}' not found"))
        })
        .collect::<Result<_>>()?;
    let target_ids: Vec<SignalId> = cfg
        .targets
        .iter()
        .map(|name| {
            base.find_signal(name)
                .ok_or_else(|| anyhow!("secure target signal '{name}' not found"))
        })
        .collect::<Result<_>>()?;

    let mut miter = Module::new(format!("secure_{}", base.name));

    // Share every public input between copy A and copy B; split secret/untrusted inputs!
    let mut shared_inputs: HashMap<SignalId, SignalId> = HashMap::new();
    for s in &base.signals {
        if s.kind == SignalKind::Input && !source_ids.contains(&s.id) {
            let id = miter.add_signal(&s.name, s.width, s.signed, SignalKind::Input);
            miter.signal_mut(id).is_clock = s.is_clock;
            shared_inputs.insert(s.id, id);
        }
    }

    let map_a = embed_copy(&mut miter, base, "a", &shared_inputs, &source_ids);
    let map_b = embed_copy(&mut miter, base, "b", &shared_inputs, &source_ids);

    // Optional declassification condition (evaluated on copy A; if true, allow divergence!).
    let mut active_en = miter.c_bool(true);
    if let Some(dec) = &cfg.declassify {
        // Prefix signal names with `a.` if needed, or resolve via temporary helper.
        let dec_prefixed = prefix_cond_signals(base, dec, "a");
        let dec_e = parse_simple_cond(&mut miter, &dec_prefixed)?;
        active_en = miter.logic_not(dec_e);
    }

    for tid in target_ids {
        let a_e = miter.sig_expr(map_a[&tid]);
        let b_e = miter.sig_expr(map_b[&tid]);
        let eq = miter.eq(a_e, b_e, false);
        miter.properties.push(Property {
            name: format!("noninterference_{}", base.signal(tid).name),
            kind: PropertyKind::Assert,
            cond: eq,
            en: active_en,
            source: None,
        });
    }

    miter.topo_sort_comb().map_err(|e| anyhow!(e))?;
    vf_ir::passes::coi_prune(&mut miter);

    let outcome = match run_kinduction(
        &miter,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )? {
        ProveOutcome::Proved { .. } => SecureOutcome::Secure,
        ProveOutcome::Fail { step, trace, .. } => SecureOutcome::Leak { step, trace },
        ProveOutcome::AssumptionsUnsat { .. } => SecureOutcome::Unknown,
        ProveOutcome::Unknown { .. } if cfg.use_pdr => match run_pdr(
            &miter,
            &PdrConfig {
                smt_solver: cfg.solver.clone(),
                ..Default::default()
            },
        ) {
            Ok(ProveOutcome::Proved { .. }) => SecureOutcome::Secure,
            Ok(ProveOutcome::Fail { step, trace, .. }) => SecureOutcome::Leak { step, trace },
            _ => SecureOutcome::Unknown,
        },
        ProveOutcome::Unknown { .. } => SecureOutcome::Unknown,
    };

    Ok((miter, outcome))
}

fn prefix_cond_signals(base: &Module, cond: &str, prefix: &str) -> String {
    // Replace signal tokens that are not shared inputs with `<prefix>.<name>`.
    let mut out = String::new();
    let mut cur = String::new();
    for ch in cond.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            cur.push(ch);
        } else {
            flush_tok(base, &mut cur, prefix, &mut out);
            out.push(ch);
        }
    }
    flush_tok(base, &mut cur, prefix, &mut out);
    out
}

fn flush_tok(base: &Module, tok: &mut String, prefix: &str, out: &mut String) {
    if tok.is_empty() {
        return;
    }
    if let Some(sid) = base.find_signal(tok) {
        if base.signal(sid).kind != SignalKind::Input {
            out.push_str(prefix);
            out.push('.');
        }
    }
    out.push_str(tok);
    tok.clear();
}

fn embed_copy(
    dst: &mut Module,
    src: &Module,
    prefix: &str,
    shared_inputs: &HashMap<SignalId, SignalId>,
    source_ids: &HashSet<SignalId>,
) -> HashMap<SignalId, SignalId> {
    let mut sig_map = HashMap::new();
    for s in &src.signals {
        let id = if let Some(&shared) = shared_inputs.get(&s.id) {
            shared
        } else {
            let kind = if s.kind == SignalKind::Output {
                SignalKind::Wire
            } else {
                s.kind
            };
            let nid = dst.add_signal(format!("{prefix}.{}", s.name), s.width, s.signed, kind);
            // If a secret/source is a register, leave its initial value unconstrained so copies A & B
            // can start with different secret values!
            if !source_ids.contains(&s.id) {
                dst.signal_mut(nid).init = s.init.clone();
            }
            nid
        };
        sig_map.insert(s.id, id);
    }

    let mut mem_map: HashMap<MemId, MemId> = HashMap::new();
    for mem in &src.memories {
        let nid = dst.add_memory(
            format!("{prefix}.{}", mem.name),
            mem.addr_width,
            mem.data_width,
            mem.depth,
        );
        dst.memory_mut(nid).init = mem.init.clone();
        mem_map.insert(mem.id, nid);
    }

    let mut expr_map: Vec<ExprId> = Vec::with_capacity(src.exprs.len());
    for node in &src.exprs {
        let kind = match &node.kind {
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
        expr_map.push(dst.intern(node.width, kind));
    }

    for ca in &src.comb {
        dst.comb.push(CombAssign {
            signal: sig_map[&ca.signal],
            value: expr_map[ca.value.0],
        });
    }
    for ff in &src.ffs {
        let init = if source_ids.contains(&ff.signal) {
            None
        } else {
            ff.init.clone()
        };
        dst.ffs.push(Ff {
            signal: sig_map[&ff.signal],
            next: expr_map[ff.next.0],
            init,
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
