use crate::{
    ClockEdge, ClockSpec, CombAssign, ConstBits, ExprId, ExprKind, Ff, Module, PropertyKind,
    SignalId, SignalKind,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChformalAction {
    Remove,
    Assert2Assume,
    Assume2Assert,
    Cover2Assert,
    Live2Fair,
    Fair2Live,
}

/// Yosys `chformal` equivalent: remove or transform formal properties on a `Module`.
pub fn chformal(m: &mut Module, filter: Option<PropertyKind>, action: ChformalAction) {
    match action {
        ChformalAction::Remove => {
            m.properties.retain(|p| match filter {
                Some(k) => p.kind != k,
                None => false,
            });
        }
        ChformalAction::Assert2Assume => {
            for p in &mut m.properties {
                if p.kind == PropertyKind::Assert {
                    p.kind = PropertyKind::Assume;
                }
            }
        }
        ChformalAction::Assume2Assert => {
            for p in &mut m.properties {
                if p.kind == PropertyKind::Assume {
                    p.kind = PropertyKind::Assert;
                }
            }
        }
        ChformalAction::Cover2Assert => {
            for p in &mut m.properties {
                if p.kind == PropertyKind::Cover {
                    p.kind = PropertyKind::Assert;
                }
            }
        }
        ChformalAction::Live2Fair => {
            for p in &mut m.properties {
                if p.kind == PropertyKind::Live {
                    p.kind = PropertyKind::Fair;
                }
            }
        }
        ChformalAction::Fair2Live => {
            for p in &mut m.properties {
                if p.kind == PropertyKind::Fair {
                    p.kind = PropertyKind::Live;
                }
            }
        }
    }
}

/// Yosys `async2sync` equivalent: convert asynchronous-reset flip-flops into synchronous
/// FFs plus a combinational reset-override output mux so synchronous formal engines see
/// asynchronous reset immediately in the same step.
pub fn async2sync(m: &mut Module) {
    let mut new_ffs = Vec::with_capacity(m.ffs.len());
    let mut new_comb = Vec::new();
    let old_ffs = std::mem::take(&mut m.ffs);

    for mut ff in old_ffs {
        if let Some(arst) = ff.async_reset.take() {
            let orig_sig = ff.signal;
            let s = m.signal(orig_sig).clone();
            let q_reg_name = format!("$async2sync_q_{}", s.name);
            let q_reg = m.add_signal(q_reg_name, s.width, s.signed, SignalKind::Reg);
            m.signal_mut(q_reg).init = ff.init.clone();

            // Effective current value of `orig_sig`: if async reset is active NOW, `reset_val`, else `q_reg`.
            let q_reg_e = m.sig_expr(q_reg);
            let effective_val = m.mux(arst.cond, arst.reset_val, q_reg_e);
            new_comb.push(CombAssign {
                signal: orig_sig,
                value: effective_val,
            });
            // On next clock edge, `q_reg` captures `reset_val` if reset active else `ff.next`.
            let next_val = m.mux(arst.cond, arst.reset_val, ff.next);
            new_ffs.push(Ff {
                signal: q_reg,
                next: next_val,
                init: ff.init,
                clock: ff.clock,
                async_reset: None,
            });
        } else {
            new_ffs.push(ff);
        }
    }

    m.ffs = new_ffs;
    m.comb.extend(new_comb);
    let _ = m.topo_sort_comb();
}

/// Yosys `clk2fflogic` / `multiclock` equivalent: lower flip-flops and memory writes with
/// explicit clock signals/edges into a single global-step transition system that updates
/// each register only on its declared clock edge (`posedge` = `!past(clk) && clk`,
/// `negedge` = `past(clk) && !clk`).
pub fn clk2fflogic(m: &mut Module) {
    // Collect distinct clock signals used across FFs and memory writes.
    let mut clk_sigs: HashSet<SignalId> = HashSet::new();
    for ff in &m.ffs {
        if let Some(c) = ff.clock {
            clk_sigs.insert(c.signal);
        }
    }
    for mw in &m.mem_writes {
        if let Some(c) = mw.clock {
            clk_sigs.insert(c.signal);
        }
    }
    if clk_sigs.is_empty() {
        return;
    }

    // Create a `$prev_clk_<name>` state register and edge-detect expressions for each clock.
    let mut past_clk_map: HashMap<SignalId, SignalId> = HashMap::new();
    let mut edge_exprs: HashMap<ClockSpec, ExprId> = HashMap::new();
    let mut sorted_clks: Vec<SignalId> = clk_sigs.into_iter().collect();
    sorted_clks.sort_by_key(|s| s.0);

    let mut clk_ffs = Vec::new();
    for clk in sorted_clks {
        let cname = m.signal(clk).name.clone();
        let prev_sig = m.add_signal(format!("$clk2ff_prev_{cname}"), 1, false, SignalKind::Reg);
        m.signal_mut(prev_sig).init = Some(ConstBits::zero(1));
        let clk_e = m.sig_expr(clk);
        let prev_e = m.sig_expr(prev_sig);
        clk_ffs.push(Ff {
            signal: prev_sig,
            next: clk_e,
            init: Some(ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });
        past_clk_map.insert(clk, prev_sig);

        let not_prev = m.not(prev_e);
        let not_clk = m.not(clk_e);
        let posedge = m.intern(1, ExprKind::And(not_prev, clk_e));
        let negedge = m.intern(1, ExprKind::And(prev_e, not_clk));
        edge_exprs.insert(
            ClockSpec {
                signal: clk,
                edge: ClockEdge::PosEdge,
            },
            posedge,
        );
        edge_exprs.insert(
            ClockSpec {
                signal: clk,
                edge: ClockEdge::NegEdge,
            },
            negedge,
        );
    }

    // Gate each clocked FF's next-state with its clock edge (holding current Q otherwise).
    let old_ffs = std::mem::take(&mut m.ffs);
    let mut new_ffs = Vec::with_capacity(old_ffs.len() + clk_ffs.len());
    for mut ff in old_ffs {
        if let Some(cspec) = ff.clock.take() {
            let edge = edge_exprs[&cspec];
            let cur_q = m.sig_expr(ff.signal);
            ff.next = m.mux(edge, ff.next, cur_q);
        }
        new_ffs.push(ff);
    }
    new_ffs.extend(clk_ffs);
    m.ffs = new_ffs;

    // Gate each clocked memory write enable with its clock edge.
    let old_mws = std::mem::take(&mut m.mem_writes);
    let mut new_mws = Vec::with_capacity(old_mws.len());
    for mut mw in old_mws {
        if let Some(cspec) = mw.clock.take() {
            let edge = edge_exprs[&cspec];
            mw.enable = m.logic_and(edge, mw.enable);
        }
        new_mws.push(mw);
    }
    m.mem_writes = new_mws;
}

/// Returns true if the module uses more than one distinct clock signal/edge across its FFs.
pub fn has_multiple_clocks(m: &Module) -> bool {
    let mut clocks: HashSet<ClockSpec> = HashSet::new();
    for ff in &m.ffs {
        if let Some(c) = ff.clock {
            clocks.insert(c);
        }
    }
    for mw in &m.mem_writes {
        if let Some(c) = mw.clock {
            clocks.insert(c);
        }
    }
    clocks.len() > 1
}

/// Cone-of-Influence (COI) reduction pass: prune combinational assigns, flip-flops, and
/// memory writes that cannot affect any active formal property (Assert/Assume/Cover/Live/Fair).
pub fn coi_prune(m: &mut Module) {
    if m.properties.is_empty() {
        return;
    }

    let comb_map: HashMap<SignalId, ExprId> = m.comb.iter().map(|c| (c.signal, c.value)).collect();
    let ff_map: HashMap<SignalId, &Ff> = m.ffs.iter().map(|f| (f.signal, f)).collect();

    let mut needed_sigs: HashSet<SignalId> = HashSet::new();
    let mut needed_mems: HashSet<crate::MemId> = HashSet::new();
    let mut work_exprs: Vec<ExprId> = Vec::new();
    let mut visited_exprs: HashSet<ExprId> = HashSet::new();

    for p in &m.properties {
        work_exprs.push(p.cond);
        work_exprs.push(p.en);
    }

    while let Some(e) = work_exprs.pop() {
        if !visited_exprs.insert(e) {
            continue;
        }
        match &m.expr(e).kind {
            ExprKind::Const(_) => {}
            ExprKind::Signal(s) => {
                if needed_sigs.insert(*s) {
                    if let Some(&val) = comb_map.get(s) {
                        work_exprs.push(val);
                    }
                    if let Some(ff) = ff_map.get(s) {
                        work_exprs.push(ff.next);
                        if let Some(ar) = &ff.async_reset {
                            work_exprs.push(ar.cond);
                            work_exprs.push(ar.reset_val);
                        }
                    }
                }
            }
            ExprKind::MemRead { mem, addr } => {
                work_exprs.push(*addr);
                if needed_mems.insert(*mem) {
                    for mw in m.mem_writes.iter().filter(|w| w.mem == *mem) {
                        work_exprs.push(mw.addr);
                        work_exprs.push(mw.data);
                        work_exprs.push(mw.enable);
                    }
                }
            }
            ExprKind::Not(a)
            | ExprKind::Neg(a)
            | ExprKind::RedAnd(a)
            | ExprKind::RedOr(a)
            | ExprKind::RedXor(a)
            | ExprKind::Slice { expr: a, .. }
            | ExprKind::Zext { expr: a, .. }
            | ExprKind::Sext { expr: a, .. } => work_exprs.push(*a),
            ExprKind::Add(a, b)
            | ExprKind::Sub(a, b)
            | ExprKind::Mul(a, b)
            | ExprKind::UDiv(a, b)
            | ExprKind::SDiv(a, b)
            | ExprKind::UMod(a, b)
            | ExprKind::SMod(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b)
            | ExprKind::Xor(a, b)
            | ExprKind::Shl(a, b)
            | ExprKind::Lshr(a, b)
            | ExprKind::Ashr(a, b)
            | ExprKind::Eq(a, b)
            | ExprKind::Ne(a, b)
            | ExprKind::Ult(a, b)
            | ExprKind::Ule(a, b)
            | ExprKind::Slt(a, b)
            | ExprKind::Sle(a, b) => {
                work_exprs.push(*a);
                work_exprs.push(*b);
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                work_exprs.push(*cond);
                work_exprs.push(*then_val);
                work_exprs.push(*else_val);
            }
            ExprKind::Concat(parts) => work_exprs.extend(parts.iter().copied()),
        }
    }

    m.comb.retain(|c| needed_sigs.contains(&c.signal));
    m.ffs.retain(|f| needed_sigs.contains(&f.signal));
    m.mem_writes.retain(|w| needed_mems.contains(&w.mem));
}
