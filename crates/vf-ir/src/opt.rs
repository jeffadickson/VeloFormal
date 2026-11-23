use crate::{ConstBits, ExprId, ExprKind, Module, SignalId};
use std::collections::HashMap;

/// Break bit-level false word cycles (e.g. ripple-carry `assign c[i+1] = f(c[i])`)
/// by substituting constant bit-slices of a combinational signal into its own definition.
pub fn resolve_self_slices(module: &mut Module) {
    for idx in 0..module.comb.len() {
        let sig = module.comb[idx].signal;
        let width = module.signal(sig).width;
        // Repeatedly extract bit-slices of `sig` from its current expression and substitute.
        for _ in 0..=width {
            let root = module.comb[idx].value;
            let bit_map = extract_bit_slices(module, root);
            if bit_map.is_empty() {
                break;
            }
            let mut cache = HashMap::new();
            let replaced = subst_sig_slices(module, root, sig, &bit_map, &mut cache);
            if replaced == root {
                break;
            }
            module.comb[idx].value = replaced;
        }
    }
}

fn extract_bit_slices(module: &Module, expr: ExprId) -> HashMap<(u32, u32), ExprId> {
    let mut map = HashMap::new();
    collect_slices(module, expr, 0, &mut map);
    map
}

fn collect_slices(
    module: &Module,
    expr: ExprId,
    lsb_offset: u32,
    map: &mut HashMap<(u32, u32), ExprId>,
) {
    let w = module.width(expr);
    map.insert((lsb_offset + w - 1, lsb_offset), expr);
    if let ExprKind::Concat(parts) = &module.expr(expr).kind {
        let mut cur_hi = lsb_offset + w;
        for &p in parts {
            let pw = module.width(p);
            cur_hi -= pw;
            collect_slices(module, p, cur_hi, map);
        }
    }
}

fn subst_sig_slices(
    module: &mut Module,
    expr: ExprId,
    target: SignalId,
    slices: &HashMap<(u32, u32), ExprId>,
    cache: &mut HashMap<ExprId, ExprId>,
) -> ExprId {
    if let Some(&r) = cache.get(&expr) {
        return r;
    }
    let node = module.expr(expr).clone();
    let res = match node.kind {
        ExprKind::Slice { expr: inner, hi, lo } => {
            if matches!(module.expr(inner).kind, ExprKind::Signal(s) if s == target) {
                if let Some(&rep) = slices.get(&(hi, lo)) {
                    // Only substitute if `rep` does not directly refer back to the exact same slice.
                    if rep != expr {
                        cache.insert(expr, rep);
                        return rep;
                    }
                }
            }
            let ni = subst_sig_slices(module, inner, target, slices, cache);
            module.slice(ni, hi, lo)
        }
        ExprKind::Const(_) | ExprKind::Signal(_) => expr,
        ExprKind::Not(a) => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.intern(node.width, ExprKind::Not(a))
        }
        ExprKind::Neg(a) => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.intern(node.width, ExprKind::Neg(a))
        }
        ExprKind::RedAnd(a) => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.intern(1, ExprKind::RedAnd(a))
        }
        ExprKind::RedOr(a) => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.intern(1, ExprKind::RedOr(a))
        }
        ExprKind::RedXor(a) => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.intern(1, ExprKind::RedXor(a))
        }
        ExprKind::Add(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Add),
        ExprKind::Sub(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Sub),
        ExprKind::Mul(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Mul),
        ExprKind::UDiv(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::UDiv),
        ExprKind::SDiv(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::SDiv),
        ExprKind::UMod(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::UMod),
        ExprKind::SMod(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::SMod),
        ExprKind::And(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::And),
        ExprKind::Or(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Or),
        ExprKind::Xor(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Xor),
        ExprKind::Shl(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Shl),
        ExprKind::Lshr(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Lshr),
        ExprKind::Ashr(a, b) => subst_bin(module, node.width, a, b, target, slices, cache, ExprKind::Ashr),
        ExprKind::Eq(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Eq),
        ExprKind::Ne(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Ne),
        ExprKind::Ult(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Ult),
        ExprKind::Ule(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Ule),
        ExprKind::Slt(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Slt),
        ExprKind::Sle(a, b) => subst_bin(module, 1, a, b, target, slices, cache, ExprKind::Sle),
        ExprKind::Mux { cond, then_val, else_val } => {
            let c = subst_sig_slices(module, cond, target, slices, cache);
            let t = subst_sig_slices(module, then_val, target, slices, cache);
            let e = subst_sig_slices(module, else_val, target, slices, cache);
            module.mux(c, t, e)
        }
        ExprKind::Concat(parts) => {
            let np: Vec<ExprId> = parts
                .into_iter()
                .map(|p| subst_sig_slices(module, p, target, slices, cache))
                .collect();
            module.concat(np)
        }
        ExprKind::Zext { expr: a, width } => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.resize(a, width, false)
        }
        ExprKind::Sext { expr: a, width } => {
            let a = subst_sig_slices(module, a, target, slices, cache);
            module.resize(a, width, true)
        }
        ExprKind::MemRead { mem, addr } => {
            let a = subst_sig_slices(module, addr, target, slices, cache);
            module.intern(node.width, ExprKind::MemRead { mem, addr: a })
        }
    };
    cache.insert(expr, res);
    res
}

fn subst_bin(
    module: &mut Module,
    width: u32,
    a: ExprId,
    b: ExprId,
    target: SignalId,
    slices: &HashMap<(u32, u32), ExprId>,
    cache: &mut HashMap<ExprId, ExprId>,
    ctor: fn(ExprId, ExprId) -> ExprKind,
) -> ExprId {
    let a = subst_sig_slices(module, a, target, slices, cache);
    let b = subst_sig_slices(module, b, target, slices, cache);
    module.intern(width, ctor(a, b))
}

/// Word-level constant folding and algebraic simplification pass (analogous to Yosys `opt_expr`).
pub fn optimize(module: &mut Module) {
    // Fold constant muxes, trivial double-negations, and constant unary/binary ops
    // by rebuilding expressions bottom-up.
    let len = module.exprs.len();
    let mut remap: Vec<ExprId> = (0..len).map(ExprId).collect();

    for i in 0..len {
        let node = module.exprs[i].clone();
        let folded = fold_node(module, node.width, &node.kind, &remap);
        remap[i] = folded;
    }

    for c in &mut module.comb {
        c.value = remap[c.value.0];
    }
    for ff in &mut module.ffs {
        ff.next = remap[ff.next.0];
        if let Some(ar) = &mut ff.async_reset {
            ar.cond = remap[ar.cond.0];
            ar.reset_val = remap[ar.reset_val.0];
        }
    }
    for mw in &mut module.mem_writes {
        mw.addr = remap[mw.addr.0];
        mw.data = remap[mw.data.0];
        mw.enable = remap[mw.enable.0];
    }
    for p in &mut module.properties {
        p.cond = remap[p.cond.0];
        p.en = remap[p.en.0];
    }
}

fn as_const(module: &Module, id: ExprId) -> Option<&ConstBits> {
    match &module.expr(id).kind {
        ExprKind::Const(c) => Some(c),
        _ => None,
    }
}

fn fold_node(module: &mut Module, width: u32, kind: &ExprKind, remap: &[ExprId]) -> ExprId {
    match kind {
        ExprKind::Const(c) => module.c_bits(c.clone()),
        ExprKind::Signal(s) => module.sig_expr(*s),
        ExprKind::Not(a) => {
            let a = remap[a.0];
            if let Some(c) = as_const(module, a).cloned() {
                let inv: String = c
                    .bits
                    .chars()
                    .map(|ch| if ch == '0' { '1' } else { '0' })
                    .collect();
                return module.c_bits(ConstBits::new(c.width, inv));
            }
            if let ExprKind::Not(inner) = module.expr(a).kind {
                return inner;
            }
            module.intern(width, ExprKind::Not(a))
        }
        ExprKind::Neg(a) => {
            let a = remap[a.0];
            if let Some(v) = as_const(module, a).and_then(|c| c.to_u128()) {
                let mask = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                return module.c_bits(ConstBits::from_u128(width, v.wrapping_neg() & mask));
            }
            module.intern(width, ExprKind::Neg(a))
        }
        ExprKind::RedAnd(a) => {
            let a = remap[a.0];
            if let Some(c) = as_const(module, a) {
                return module.c_bool(c.is_all_ones());
            }
            module.intern(1, ExprKind::RedAnd(a))
        }
        ExprKind::RedOr(a) => {
            let a = remap[a.0];
            if let Some(c) = as_const(module, a) {
                return module.c_bool(!c.is_zero());
            }
            module.intern(1, ExprKind::RedOr(a))
        }
        ExprKind::RedXor(a) => {
            let a = remap[a.0];
            if let Some(c) = as_const(module, a) {
                let ones = c.bits.bytes().filter(|&b| b == b'1').count();
                return module.c_bool(ones % 2 == 1);
            }
            module.intern(1, ExprKind::RedXor(a))
        }
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let cond = remap[cond.0];
            let then_val = remap[then_val.0];
            let else_val = remap[else_val.0];
            module.mux(cond, then_val, else_val)
        }
        ExprKind::And(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if let Some(c) = as_const(module, a) {
                if c.is_zero() {
                    return module.c_bits(ConstBits::zero(width));
                }
                if c.is_all_ones() {
                    return b;
                }
            }
            if let Some(c) = as_const(module, b) {
                if c.is_zero() {
                    return module.c_bits(ConstBits::zero(width));
                }
                if c.is_all_ones() {
                    return a;
                }
            }
            module.intern(width, ExprKind::And(a, b))
        }
        ExprKind::Or(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if let Some(c) = as_const(module, a) {
                if c.is_zero() {
                    return b;
                }
                if c.is_all_ones() {
                    return module.c_bits(ConstBits::new(width, "1".repeat(width as usize)));
                }
            }
            if let Some(c) = as_const(module, b) {
                if c.is_zero() {
                    return a;
                }
                if c.is_all_ones() {
                    return module.c_bits(ConstBits::new(width, "1".repeat(width as usize)));
                }
            }
            module.intern(width, ExprKind::Or(a, b))
        }
        ExprKind::Xor(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if let Some(c) = as_const(module, a) {
                if c.is_zero() {
                    return b;
                }
            }
            if let Some(c) = as_const(module, b) {
                if c.is_zero() {
                    return a;
                }
            }
            module.intern(width, ExprKind::Xor(a, b))
        }
        ExprKind::Add(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if let (Some(va), Some(vb)) = (
                as_const(module, a).and_then(|c| c.to_u128()),
                as_const(module, b).and_then(|c| c.to_u128()),
            ) {
                let mask = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                return module.c_bits(ConstBits::from_u128(width, va.wrapping_add(vb) & mask));
            }
            module.intern(width, ExprKind::Add(a, b))
        }
        ExprKind::Sub(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if let (Some(va), Some(vb)) = (
                as_const(module, a).and_then(|c| c.to_u128()),
                as_const(module, b).and_then(|c| c.to_u128()),
            ) {
                let mask = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                return module.c_bits(ConstBits::from_u128(width, va.wrapping_sub(vb) & mask));
            }
            module.intern(width, ExprKind::Sub(a, b))
        }
        ExprKind::Mul(a, b) => module.intern(width, ExprKind::Mul(remap[a.0], remap[b.0])),
        ExprKind::UDiv(a, b) => module.intern(width, ExprKind::UDiv(remap[a.0], remap[b.0])),
        ExprKind::SDiv(a, b) => module.intern(width, ExprKind::SDiv(remap[a.0], remap[b.0])),
        ExprKind::UMod(a, b) => module.intern(width, ExprKind::UMod(remap[a.0], remap[b.0])),
        ExprKind::SMod(a, b) => module.intern(width, ExprKind::SMod(remap[a.0], remap[b.0])),
        ExprKind::Shl(a, b) => module.intern(width, ExprKind::Shl(remap[a.0], remap[b.0])),
        ExprKind::Lshr(a, b) => module.intern(width, ExprKind::Lshr(remap[a.0], remap[b.0])),
        ExprKind::Ashr(a, b) => module.intern(width, ExprKind::Ashr(remap[a.0], remap[b.0])),
        ExprKind::Eq(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if a == b {
                return module.c_bool(true);
            }
            if let (Some(ca), Some(cb)) = (as_const(module, a), as_const(module, b)) {
                return module.c_bool(ca == cb);
            }
            module.intern(1, ExprKind::Eq(a, b))
        }
        ExprKind::Ne(a, b) => {
            let a = remap[a.0];
            let b = remap[b.0];
            if a == b {
                return module.c_bool(false);
            }
            if let (Some(ca), Some(cb)) = (as_const(module, a), as_const(module, b)) {
                return module.c_bool(ca != cb);
            }
            module.intern(1, ExprKind::Ne(a, b))
        }
        ExprKind::Ult(a, b) => module.intern(1, ExprKind::Ult(remap[a.0], remap[b.0])),
        ExprKind::Ule(a, b) => module.intern(1, ExprKind::Ule(remap[a.0], remap[b.0])),
        ExprKind::Slt(a, b) => module.intern(1, ExprKind::Slt(remap[a.0], remap[b.0])),
        ExprKind::Sle(a, b) => module.intern(1, ExprKind::Sle(remap[a.0], remap[b.0])),
        ExprKind::Concat(parts) => {
            let mapped: Vec<ExprId> = parts.iter().map(|p| remap[p.0]).collect();
            if mapped.iter().all(|&p| as_const(module, p).is_some()) {
                let mut bits = String::new();
                for &p in &mapped {
                    bits.push_str(&as_const(module, p).unwrap().bits);
                }
                return module.c_bits(ConstBits::new(width, bits));
            }
            module.concat(mapped)
        }
        ExprKind::Slice { expr, hi, lo } => {
            let e = remap[expr.0];
            if let Some(c) = as_const(module, e).cloned() {
                let start = (c.width - 1 - hi) as usize;
                let end = (c.width - lo) as usize;
                return module.c_bits(ConstBits::new(hi - lo + 1, &c.bits[start..end]));
            }
            module.slice(e, *hi, *lo)
        }
        ExprKind::Zext { expr, width: w } => {
            let e = remap[expr.0];
            if let Some(c) = as_const(module, e).cloned() {
                return module.c_bits(c.resize_zext(*w));
            }
            module.resize(e, *w, false)
        }
        ExprKind::Sext { expr, width: w } => {
            let e = remap[expr.0];
            if let Some(c) = as_const(module, e).cloned() {
                return module.c_bits(c.resize_sext(*w));
            }
            module.resize(e, *w, true)
        }
        ExprKind::MemRead { mem, addr } => {
            let addr = remap[addr.0];
            module.intern(width, ExprKind::MemRead { mem: *mem, addr })
        }
    }
}
