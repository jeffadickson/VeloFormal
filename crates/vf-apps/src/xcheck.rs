use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{
    AsyncReset, CombAssign, ConstBits, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property,
    PropertyKind, SignalId, SignalKind,
};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct XCheckConfig {
    /// Signals whose X-freedom should be proved (defaults to all module outputs if empty).
    pub targets: Vec<String>,
    /// Additional primary inputs / signals allowed to carry X values.
    pub x_sources: Vec<String>,
    /// Number of initial cycles to skip checking X-freedom (e.g. reset cycles).
    pub skip_cycles: usize,
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for XCheckConfig {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            x_sources: Vec::new(),
            skip_cycles: 0,
            depth: 15,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum XStatus {
    XFree,
    XLeak { step: usize, trace: Vec<TraceFrame> },
    Unknown,
}

#[derive(Debug, Clone)]
pub struct XCheckResult {
    pub signal: String,
    pub status: XStatus,
}

/// Questa X-Check equivalent: 4-state formal X-propagation verification via dual-rail
/// `(val, is_x)` encoding.
///
/// Uninitialized flip-flops start with `is_x = all 1s` (modeling silicon power-up X).
/// Controlling values (`0 & X = 0`, `1 | X = 1`) and IEEE 1800 bit-merging ternary muxes
/// are modeled accurately, and every target output is checked for X-freedom.
pub fn run_xcheck(base: &Module, cfg: &XCheckConfig) -> Result<(Module, Vec<XCheckResult>)> {
    let target_ids: Vec<SignalId> = if cfg.targets.is_empty() {
        base.signals
            .iter()
            .filter(|s| s.kind == SignalKind::Output)
            .map(|s| s.id)
            .collect()
    } else {
        cfg.targets
            .iter()
            .map(|name| {
                base.find_signal(name)
                    .ok_or_else(|| anyhow!("xcheck target signal '{name}' not found"))
            })
            .collect::<Result<_>>()?
    };

    let x_source_set: HashSet<String> = cfg.x_sources.iter().cloned().collect();
    let (dual, x_sig_map) = build_dual_rail_module(base, &x_source_set, cfg.skip_cycles)?;

    let mut results = Vec::with_capacity(target_ids.len());
    for tid in target_ids {
        let sig_name = base.signal(tid).name.clone();
        let x_sid = x_sig_map[&tid];
        let mut sub = dual.clone();
        sub.properties.retain(|p| p.kind == PropertyKind::Assume);

        let x_expr = sub.sig_expr(x_sid);
        let zero = sub.c_bits(ConstBits::zero(sub.signal(x_sid).width));
        let x_free = sub.eq(x_expr, zero, false);
        let en = if let Some(ready_sig) = sub.find_signal("$xcheck_ready") {
            sub.sig_expr(ready_sig)
        } else {
            sub.c_bool(true)
        };
        sub.properties.push(Property {
            name: format!("xfree_{sig_name}"),
            kind: PropertyKind::Assert,
            cond: x_free,
            en,
            source: None,
        });
        vf_ir::passes::coi_prune(&mut sub);

        let status = match run_kinduction(
            &sub,
            &ProveConfig {
                max_k: cfg.depth.min(8),
                solver: cfg.solver.clone(),
            },
        )? {
            ProveOutcome::Proved { .. } => XStatus::XFree,
            ProveOutcome::Fail { step, trace, .. } => XStatus::XLeak { step, trace },
            ProveOutcome::AssumptionsUnsat { .. } => XStatus::Unknown,
            ProveOutcome::Unknown { .. } if cfg.use_pdr => match run_pdr(
                &sub,
                &PdrConfig {
                    smt_solver: cfg.solver.clone(),
                    ..Default::default()
                },
            ) {
                Ok(ProveOutcome::Proved { .. }) => XStatus::XFree,
                Ok(ProveOutcome::Fail { step, trace, .. }) => XStatus::XLeak { step, trace },
                _ => XStatus::Unknown,
            },
            ProveOutcome::Unknown { .. } => XStatus::Unknown,
        };
        results.push(XCheckResult {
            signal: sig_name,
            status,
        });
    }

    Ok((dual, results))
}

fn build_dual_rail_module(
    src: &Module,
    x_sources: &HashSet<String>,
    skip_cycles: usize,
) -> Result<(Module, HashMap<SignalId, SignalId>)> {
    let mut dst = Module::new(format!("xcheck_{}", src.name));
    let mut v_sig_map: HashMap<SignalId, SignalId> = HashMap::new();
    let mut x_sig_map: HashMap<SignalId, SignalId> = HashMap::new();

    let ff_inits: HashMap<SignalId, bool> = src
        .ffs
        .iter()
        .map(|f| (f.signal, f.init.is_some()))
        .collect();

    for s in &src.signals {
        let vid = dst.add_signal(&s.name, s.width, s.signed, s.kind);
        dst.signal_mut(vid).is_clock = s.is_clock;
        dst.signal_mut(vid).init = s.init.clone();
        v_sig_map.insert(s.id, vid);

        let is_x_src = x_sources.contains(&s.name);
        let is_ff = ff_inits.contains_key(&s.id);
        let x_kind = if is_ff {
            SignalKind::Reg
        } else {
            match s.kind {
                SignalKind::Input if is_x_src => SignalKind::Input,
                SignalKind::Input | SignalKind::AnySeq | SignalKind::AnyConst => SignalKind::Wire,
                SignalKind::Output | SignalKind::Wire => SignalKind::Wire,
                SignalKind::Reg => SignalKind::Reg,
            }
        };
        let xid = dst.add_signal(format!("{}__x", s.name), s.width, false, x_kind);
        if is_ff || s.kind == SignalKind::Reg {
            let has_init = s.init.is_some() || ff_inits.get(&s.id).copied().unwrap_or(false);
            let x_init = if !has_init || is_x_src {
                all_ones(s.width)
            } else {
                ConstBits::zero(s.width)
            };
            dst.signal_mut(xid).init = Some(x_init);
        } else {
            match s.kind {
                SignalKind::Input if is_x_src => {}
                SignalKind::Input | SignalKind::AnySeq | SignalKind::AnyConst => {
                    let z = if is_x_src {
                        dst.c_bits(all_ones(s.width))
                    } else {
                        dst.c_bits(ConstBits::zero(s.width))
                    };
                    dst.comb.push(CombAssign {
                        signal: xid,
                        value: z,
                    });
                }
                SignalKind::Output | SignalKind::Wire | SignalKind::Reg => {}
            }
        }
        x_sig_map.insert(s.id, xid);
    }

    let mut v_mem_map: HashMap<MemId, MemId> = HashMap::new();
    let mut x_mem_map: HashMap<MemId, MemId> = HashMap::new();
    for mem in &src.memories {
        let vm = dst.add_memory(&mem.name, mem.addr_width, mem.data_width, mem.depth);
        dst.memory_mut(vm).init = mem.init.clone();
        v_mem_map.insert(mem.id, vm);

        let xm = dst.add_memory(
            format!("{}__x", mem.name),
            mem.addr_width,
            mem.data_width,
            mem.depth,
        );
        // Any explicitly initialized memory address starts with X=0; uninitialized addresses start all-1s!
        let init_addrs: HashSet<u32> = mem.init.iter().map(|(a, _)| *a).collect();
        if mem.depth <= 256 {
            let ones = all_ones(mem.data_width);
            let zeros = ConstBits::zero(mem.data_width);
            for a in 0..mem.depth {
                dst.memory_mut(xm).init.push((
                    a,
                    if init_addrs.contains(&a) {
                        zeros.clone()
                    } else {
                        ones.clone()
                    },
                ));
            }
        }
        x_mem_map.insert(mem.id, xm);
    }

    // Dual-rail expression translation: returns `(val_expr, x_expr)` for each `src.exprs[i]`.
    let mut expr_map: Vec<(ExprId, ExprId)> = Vec::with_capacity(src.exprs.len());
    for node in &src.exprs {
        let w = node.width;
        let pair = match &node.kind {
            ExprKind::Const(c) => (
                dst.c_bits(c.clone()),
                dst.c_bits(ConstBits::zero(c.width)),
            ),
            ExprKind::Signal(s) => (dst.sig_expr(v_sig_map[s]), dst.sig_expr(x_sig_map[s])),
            ExprKind::Not(a) => {
                let (av, ax) = expr_map[a.0];
                (dst.intern(w, ExprKind::Not(av)), ax)
            }
            ExprKind::Neg(a) => {
                let (av, ax) = expr_map[a.0];
                let any_x = dst.to_bool(ax);
                let x = broadcast_x(&mut dst, any_x, w);
                (dst.intern(w, ExprKind::Neg(av)), x)
            }
            ExprKind::And(a, b) => {
                let (av, ax) = expr_map[a.0];
                let (bv, bx) = expr_map[b.0];
                let v = dst.intern(w, ExprKind::And(av, bv));
                // Known 0 dominates X: `known0_a = ~av & ~ax`.
                let nav = dst.not(av);
                let nax = dst.not(ax);
                let k0a = dst.intern(w, ExprKind::And(nav, nax));
                let nbv = dst.not(bv);
                let nbx = dst.not(bx);
                let k0b = dst.intern(w, ExprKind::And(nbv, nbx));
                let any_k0 = dst.intern(w, ExprKind::Or(k0a, k0b));
                let not_k0 = dst.not(any_k0);
                let either_x = dst.intern(w, ExprKind::Or(ax, bx));
                let x = dst.intern(w, ExprKind::And(either_x, not_k0));
                (v, x)
            }
            ExprKind::Or(a, b) => {
                let (av, ax) = expr_map[a.0];
                let (bv, bx) = expr_map[b.0];
                let v = dst.intern(w, ExprKind::Or(av, bv));
                // Known 1 dominates X: `known1_a = av & ~ax`.
                let nax = dst.not(ax);
                let k1a = dst.intern(w, ExprKind::And(av, nax));
                let nbx = dst.not(bx);
                let k1b = dst.intern(w, ExprKind::And(bv, nbx));
                let any_k1 = dst.intern(w, ExprKind::Or(k1a, k1b));
                let not_k1 = dst.not(any_k1);
                let either_x = dst.intern(w, ExprKind::Or(ax, bx));
                let x = dst.intern(w, ExprKind::And(either_x, not_k1));
                (v, x)
            }
            ExprKind::Xor(a, b) => {
                let (av, ax) = expr_map[a.0];
                let (bv, bx) = expr_map[b.0];
                (
                    dst.intern(w, ExprKind::Xor(av, bv)),
                    dst.intern(w, ExprKind::Or(ax, bx)),
                )
            }
            ExprKind::RedOr(a) => {
                let (av, ax) = expr_map[a.0];
                let aw = dst.width(av);
                let v = dst.intern(1, ExprKind::RedOr(av));
                let nax = dst.not(ax);
                let k1 = dst.intern(aw, ExprKind::And(av, nax));
                let has_k1 = dst.intern(1, ExprKind::RedOr(k1));
                let not_has_k1 = dst.not(has_k1);
                let has_x = dst.intern(1, ExprKind::RedOr(ax));
                let x = dst.intern(1, ExprKind::And(has_x, not_has_k1));
                (v, x)
            }
            ExprKind::RedAnd(a) => {
                let (av, ax) = expr_map[a.0];
                let aw = dst.width(av);
                let v = dst.intern(1, ExprKind::RedAnd(av));
                let nav = dst.not(av);
                let nax = dst.not(ax);
                let k0 = dst.intern(aw, ExprKind::And(nav, nax));
                let has_k0 = dst.intern(1, ExprKind::RedOr(k0));
                let not_has_k0 = dst.not(has_k0);
                let has_x = dst.intern(1, ExprKind::RedOr(ax));
                let x = dst.intern(1, ExprKind::And(has_x, not_has_k0));
                (v, x)
            }
            ExprKind::RedXor(a) => {
                let (av, ax) = expr_map[a.0];
                (
                    dst.intern(1, ExprKind::RedXor(av)),
                    dst.intern(1, ExprKind::RedOr(ax)),
                )
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                let (cv, cx) = expr_map[cond.0];
                let (tv, tx) = expr_map[then_val.0];
                let (ev, ex) = expr_map[else_val.0];
                let v = dst.mux(cv, tv, ev);
                let normal_x = dst.mux(cv, tx, ex);
                let diff = dst.intern(w, ExprKind::Xor(tv, ev));
                let tx_or_ex = dst.intern(w, ExprKind::Or(tx, ex));
                let merged_x = dst.intern(w, ExprKind::Or(tx_or_ex, diff));
                let x = dst.mux(cx, merged_x, normal_x);
                (v, x)
            }
            ExprKind::Slice { expr, hi, lo } => {
                let (ev, ex) = expr_map[expr.0];
                (dst.slice(ev, *hi, *lo), dst.slice(ex, *hi, *lo))
            }
            ExprKind::Concat(parts) => {
                let vs: Vec<ExprId> = parts.iter().map(|p| expr_map[p.0].0).collect();
                let xs: Vec<ExprId> = parts.iter().map(|p| expr_map[p.0].1).collect();
                (dst.concat(vs), dst.concat(xs))
            }
            ExprKind::Zext { expr, width } => {
                let (ev, ex) = expr_map[expr.0];
                (
                    dst.intern(*width, ExprKind::Zext { expr: ev, width: *width }),
                    dst.intern(*width, ExprKind::Zext { expr: ex, width: *width }),
                )
            }
            ExprKind::Sext { expr, width } => {
                let (ev, ex) = expr_map[expr.0];
                (
                    dst.intern(*width, ExprKind::Sext { expr: ev, width: *width }),
                    dst.intern(*width, ExprKind::Sext { expr: ex, width: *width }),
                )
            }
            ExprKind::MemRead { mem, addr } => {
                let (av, ax) = expr_map[addr.0];
                let v = dst.intern(
                    w,
                    ExprKind::MemRead {
                        mem: v_mem_map[mem],
                        addr: av,
                    },
                );
                let data_x = dst.intern(
                    w,
                    ExprKind::MemRead {
                        mem: x_mem_map[mem],
                        addr: av,
                    },
                );
                let addr_has_x = dst.to_bool(ax);
                let all1 = dst.c_bits(all_ones(w));
                let x = dst.mux(addr_has_x, all1, data_x);
                (v, x)
            }
            // Binary arithmetic, shift, and relational ops: any X bit makes result all-X (IEEE 1800).
            ExprKind::Add(a, b)
            | ExprKind::Sub(a, b)
            | ExprKind::Mul(a, b)
            | ExprKind::UDiv(a, b)
            | ExprKind::SDiv(a, b)
            | ExprKind::UMod(a, b)
            | ExprKind::SMod(a, b)
            | ExprKind::Shl(a, b)
            | ExprKind::Lshr(a, b)
            | ExprKind::Ashr(a, b)
            | ExprKind::Eq(a, b)
            | ExprKind::Ne(a, b)
            | ExprKind::Ult(a, b)
            | ExprKind::Ule(a, b)
            | ExprKind::Slt(a, b)
            | ExprKind::Sle(a, b) => {
                let (av, ax) = expr_map[a.0];
                let (bv, bx) = expr_map[b.0];
                let v_kind = match &node.kind {
                    ExprKind::Add(_, _) => ExprKind::Add(av, bv),
                    ExprKind::Sub(_, _) => ExprKind::Sub(av, bv),
                    ExprKind::Mul(_, _) => ExprKind::Mul(av, bv),
                    ExprKind::UDiv(_, _) => ExprKind::UDiv(av, bv),
                    ExprKind::SDiv(_, _) => ExprKind::SDiv(av, bv),
                    ExprKind::UMod(_, _) => ExprKind::UMod(av, bv),
                    ExprKind::SMod(_, _) => ExprKind::SMod(av, bv),
                    ExprKind::Shl(_, _) => ExprKind::Shl(av, bv),
                    ExprKind::Lshr(_, _) => ExprKind::Lshr(av, bv),
                    ExprKind::Ashr(_, _) => ExprKind::Ashr(av, bv),
                    ExprKind::Eq(_, _) => ExprKind::Eq(av, bv),
                    ExprKind::Ne(_, _) => ExprKind::Ne(av, bv),
                    ExprKind::Ult(_, _) => ExprKind::Ult(av, bv),
                    ExprKind::Ule(_, _) => ExprKind::Ule(av, bv),
                    ExprKind::Slt(_, _) => ExprKind::Slt(av, bv),
                    ExprKind::Sle(_, _) => ExprKind::Sle(av, bv),
                    _ => unreachable!(),
                };
                let v = dst.intern(w, v_kind);
                let ax_any = dst.to_bool(ax);
                let bx_any = dst.to_bool(bx);
                let any_x = dst.logic_or(ax_any, bx_any);
                let x = broadcast_x(&mut dst, any_x, w);
                (v, x)
            }
        };
        expr_map.push(pair);
    }

    for ca in &src.comb {
        let (v, x) = expr_map[ca.value.0];
        dst.comb.push(CombAssign {
            signal: v_sig_map[&ca.signal],
            value: v,
        });
        dst.comb.push(CombAssign {
            signal: x_sig_map[&ca.signal],
            value: x,
        });
    }

    for ff in &src.ffs {
        let (nv, nx) = expr_map[ff.next.0];
        let clk = ff.clock.map(|c| vf_ir::ClockSpec {
            signal: v_sig_map[&c.signal],
            edge: c.edge,
        });
        dst.ffs.push(Ff {
            signal: v_sig_map[&ff.signal],
            next: nv,
            init: ff.init.clone(),
            clock: clk,
            async_reset: ff.async_reset.as_ref().map(|r| AsyncReset {
                cond: expr_map[r.cond.0].0,
                reset_val: expr_map[r.reset_val.0].0,
            }),
        });

        let x_init = dst.signal(x_sig_map[&ff.signal]).init.clone();
        dst.ffs.push(Ff {
            signal: x_sig_map[&ff.signal],
            next: nx,
            init: x_init,
            clock: clk,
            async_reset: ff.async_reset.as_ref().map(|r| AsyncReset {
                cond: expr_map[r.cond.0].0,
                reset_val: expr_map[r.reset_val.0].1,
            }),
        });
    }

    for mw in &src.mem_writes {
        let (av, ax) = expr_map[mw.addr.0];
        let (dv, dx) = expr_map[mw.data.0];
        let (ev, ex) = expr_map[mw.enable.0];
        let clk = mw.clock.map(|c| vf_ir::ClockSpec {
            signal: v_sig_map[&c.signal],
            edge: c.edge,
        });
        dst.mem_writes.push(MemWrite {
            mem: v_mem_map[&mw.mem],
            addr: av,
            data: dv,
            enable: ev,
            clock: clk,
        });
        // If write-enable or write-address is X, mark written data X as well.
        let dw = dst.width(dx);
        let addr_x = dst.to_bool(ax);
        let en_or_addr_x = dst.logic_or(ex, addr_x);
        let all1 = dst.c_bits(all_ones(dw));
        let eff_dx = dst.mux(en_or_addr_x, all1, dx);
        let eff_en = dst.logic_or(ev, ex);
        dst.mem_writes.push(MemWrite {
            mem: x_mem_map[&mw.mem],
            addr: av,
            data: eff_dx,
            enable: eff_en,
            clock: clk,
        });
    }

    for p in &src.properties {
        if p.kind == PropertyKind::Assume {
            dst.properties.push(Property {
                name: p.name.clone(),
                kind: PropertyKind::Assume,
                cond: expr_map[p.cond.0].0,
                en: expr_map[p.en.0].0,
                source: p.source.clone(),
            });
        }
    }

    // Add a saturating cycle counter if `skip_cycles > 0`.
    if skip_cycles > 0 {
        let cnt_w = (usize::BITS - skip_cycles.leading_zeros()).max(2);
        let cnt = dst.add_signal("$xcheck_cnt", cnt_w, false, SignalKind::Reg);
        dst.signal_mut(cnt).init = Some(ConstBits::zero(cnt_w));
        let cnt_e = dst.sig_expr(cnt);
        let limit = dst.c_u64(cnt_w, skip_cycles as u64);
        let done = dst.intern(1, ExprKind::Ule(limit, cnt_e));
        let one = dst.c_u64(cnt_w, 1);
        let inc = dst.intern(cnt_w, ExprKind::Add(cnt_e, one));
        let next_cnt = dst.mux(done, cnt_e, inc);
        dst.ffs.push(Ff {
            signal: cnt,
            next: next_cnt,
            init: Some(ConstBits::zero(cnt_w)),
            clock: None,
            async_reset: None,
        });
        let ready = dst.add_signal("$xcheck_ready", 1, false, SignalKind::Wire);
        dst.comb.push(CombAssign {
            signal: ready,
            value: done,
        });
    }

    dst.topo_sort_comb().map_err(|e| anyhow!(e))?;
    Ok((dst, x_sig_map))
}

fn all_ones(width: u32) -> ConstBits {
    ConstBits::new(width, "1".repeat(width as usize))
}

fn broadcast_x(m: &mut Module, is_x: ExprId, width: u32) -> ExprId {
    if width == 1 {
        is_x
    } else {
        let ones = m.c_bits(all_ones(width));
        let zeros = m.c_bits(ConstBits::zero(width));
        m.mux(is_x, ones, zeros)
    }
}
