use anyhow::{bail, Result};
use std::collections::HashMap;
use vf_ir::{
    AsyncReset, CombAssign, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property, PropertyKind,
    SignalId, SignalKind,
};

use crate::kind::{run_kinduction, ProveConfig, ProveOutcome};

/// Build a miter module comparing all outputs of `gold` and `gate` under shared inputs.
pub fn build_miter(gold: &Module, gate: &Module) -> Result<Module> {
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

pub fn check_equiv(gold: &Module, gate: &Module, cfg: &ProveConfig) -> Result<(Module, ProveOutcome)> {
    let miter = build_miter(gold, gate)?;
    let outcome = run_kinduction(&miter, cfg)?;
    Ok((miter, outcome))
}
