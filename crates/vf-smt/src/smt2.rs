use std::collections::HashMap;
use std::fmt::Write;
use vf_ir::{ConstBits, ExprId, ExprKind, MemId, Module, PropertyKind, SignalId, SignalKind};

pub fn sig_name(id: SignalId, step: usize) -> String {
    format!("s{}_t{}", id.0, step)
}

pub fn mem_name(id: MemId, step: usize) -> String {
    format!("m{}_t{}", id.0, step)
}

pub fn smt_const(c: &ConstBits) -> String {
    format!("#b{}", c.bits)
}

pub fn header(m: &Module) -> String {
    let logic = if m.memories.is_empty() {
        "QF_BV"
    } else {
        "QF_ABV"
    };
    format!("(set-logic {logic})\n(set-option :produce-models true)\n")
}

/// Emit SMT-LIB2 declarations and constraints for a single unrolled time step `step`.
///
/// - If `apply_init` is true (typically `step == 0` in BMC/cover/base-case), initial values
///   for FFs and memories are asserted.
/// - If `trans_from` is `Some(prev)`, next-state equations from `prev -> step` are asserted,
///   and `AnyConst` signals are constrained equal to their value at `prev`.
pub fn encode_step(
    m: &Module,
    step: usize,
    apply_init: bool,
    trans_from: Option<usize>,
) -> String {
    let mut out = String::new();

    // 1. Declare all signal variables for this step.
    for s in &m.signals {
        let _ = writeln!(
            out,
            "(declare-fun {} () (_ BitVec {}))",
            sig_name(s.id, step),
            s.width
        );
    }

    // 2. Declare all memory arrays for this step.
    for mem in &m.memories {
        let _ = writeln!(
            out,
            "(declare-fun {} () (Array (_ BitVec {}) (_ BitVec {})))",
            mem_name(mem.id, step),
            mem.addr_width,
            mem.data_width
        );
    }

    // 3. Emit cached subexpressions for this step as let/define-const aliases (DAG sharing!).
    //    We emit expressions referenced by comb, properties, and (when trans_from is Some) prev-step FFs.
    let mut expr_names: HashMap<ExprId, String> = HashMap::new();
    let mut define_expr = |out: &mut String, id: ExprId, s: usize| -> String {
        emit_expr_dag(m, id, s, &mut expr_names, out)
    };

    // 4. Combinational assignments (already topologically sorted!).
    for ca in &m.comb {
        let rhs = define_expr(&mut out, ca.value, step);
        let _ = writeln!(out, "(assert (= {} {rhs}))", sig_name(ca.signal, step));
    }

    // 5. Initial state constraints.
    if apply_init {
        for ff in &m.ffs {
            if let Some(init) = &ff.init {
                let _ = writeln!(
                    out,
                    "(assert (= {} {}))",
                    sig_name(ff.signal, step),
                    smt_const(init)
                );
            }
        }
        for mem in &m.memories {
            for (addr, val) in &mem.init {
                let addr_bits = ConstBits::from_u64(mem.addr_width, *addr as u64);
                let _ = writeln!(
                    out,
                    "(assert (= (select {} {}) {}))",
                    mem_name(mem.id, step),
                    smt_const(&addr_bits),
                    smt_const(val)
                );
            }
        }
    }

    // 6. State transitions from `prev -> step`.
    if let Some(prev) = trans_from {
        // AnyConst signals stay constant across cycles.
        for s in &m.signals {
            if s.kind == SignalKind::AnyConst {
                let _ = writeln!(
                    out,
                    "(assert (= {} {}))",
                    sig_name(s.id, step),
                    sig_name(s.id, prev)
                );
            }
        }

        // Flip-flop transitions (evaluated in `prev` frame!).
        let mut prev_cache: HashMap<ExprId, String> = HashMap::new();
        for ff in &m.ffs {
            let next_expr = emit_expr_dag(m, ff.next, prev, &mut prev_cache, &mut out);
            let _ = writeln!(
                out,
                "(assert (= {} {next_expr}))",
                sig_name(ff.signal, step)
            );
        }

        // Memory transitions (apply all writes to each memory sequentially in `prev` frame).
        for mem in &m.memories {
            let mut cur = mem_name(mem.id, prev);
            for mw in m.mem_writes.iter().filter(|w| w.mem == mem.id) {
                let addr = emit_expr_dag(m, mw.addr, prev, &mut prev_cache, &mut out);
                let data = emit_expr_dag(m, mw.data, prev, &mut prev_cache, &mut out);
                let en = emit_expr_dag(m, mw.enable, prev, &mut prev_cache, &mut out);
                cur = format!("(ite (= {en} #b1) (store {cur} {addr} {data}) {cur})");
            }
            let _ = writeln!(out, "(assert (= {} {cur}))", mem_name(mem.id, step));
        }
    }

    out
}

pub fn encode_assumes(m: &Module, step: usize) -> String {
    let mut out = String::new();
    let mut cache = HashMap::new();
    for p in &m.properties {
        if p.kind == PropertyKind::Assume {
            let en = emit_expr_dag(m, p.en, step, &mut cache, &mut out);
            let cond = emit_expr_dag(m, p.cond, step, &mut cache, &mut out);
            let _ = writeln!(out, "(assert (=> (= {en} #b1) (= {cond} #b1)))");
        }
    }
    out
}

pub fn encode_asserts_hold(m: &Module, step: usize) -> String {
    let mut out = String::new();
    let mut cache = HashMap::new();
    for p in &m.properties {
        if p.kind == PropertyKind::Assert {
            let en = emit_expr_dag(m, p.en, step, &mut cache, &mut out);
            let cond = emit_expr_dag(m, p.cond, step, &mut cache, &mut out);
            let _ = writeln!(out, "(assert (=> (= {en} #b1) (= {cond} #b1)))");
        }
    }
    out
}

/// Emit a single assertion-failure formula `(and (= en #b1) (= cond #b0))` as a defined Bool.
pub fn encode_assert_failure(m: &Module, prop_idx: usize, step: usize) -> (String, String) {
    let mut defs = String::new();
    let mut cache = HashMap::new();
    let p = &m.properties[prop_idx];
    let en = emit_expr_dag(m, p.en, step, &mut cache, &mut defs);
    let cond = emit_expr_dag(m, p.cond, step, &mut cache, &mut defs);
    let formula = format!("(and (= {en} #b1) (= {cond} #b0))");
    (defs, formula)
}

/// Emit a single cover-hit formula `(and (= en #b1) (= cond #b1))`.
pub fn encode_cover_hit(m: &Module, prop_idx: usize, step: usize) -> (String, String) {
    let mut defs = String::new();
    let mut cache = HashMap::new();
    let p = &m.properties[prop_idx];
    let en = emit_expr_dag(m, p.en, step, &mut cache, &mut defs);
    let cond = emit_expr_dag(m, p.cond, step, &mut cache, &mut defs);
    let formula = format!("(and (= {en} #b1) (= {cond} #b1))");
    (defs, formula)
}

/// Dump a readable 2-step standalone SMT-LIB2 file representing the transition system.
pub fn dump_standalone_smt2(m: &Module) -> String {
    let mut out = header(m);
    let _ = writeln!(out, "; Module: {}", m.name);
    let _ = writeln!(out, "; Step 0 (initial state)");
    out.push_str(&encode_step(m, 0, true, None));
    out.push_str(&encode_assumes(m, 0));
    let _ = writeln!(out, "; Step 1 (transition)");
    out.push_str(&encode_step(m, 1, false, Some(0)));
    out.push_str(&encode_assumes(m, 1));
    out
}

fn emit_expr_dag(
    m: &Module,
    id: ExprId,
    step: usize,
    cache: &mut HashMap<ExprId, String>,
    _defs: &mut String,
) -> String {
    if let Some(s) = cache.get(&id) {
        return s.clone();
    }
    let res = match &m.expr(id).kind {
        ExprKind::Const(c) => smt_const(c),
        ExprKind::Signal(s) => sig_name(*s, step),
        ExprKind::Not(a) => {
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            format!("(bvnot {a})")
        }
        ExprKind::Neg(a) => {
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            format!("(bvneg {a})")
        }
        ExprKind::RedAnd(a) => {
            let w = m.width(*a);
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            let ones = "1".repeat(w as usize);
            format!("(ite (= {a} #b{ones}) #b1 #b0)")
        }
        ExprKind::RedOr(a) => {
            let w = m.width(*a);
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            let zeros = "0".repeat(w as usize);
            format!("(ite (= {a} #b{zeros}) #b0 #b1)")
        }
        ExprKind::RedXor(a) => {
            let w = m.width(*a);
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            if w == 1 {
                a
            } else {
                let mut acc = format!("((_ extract 0 0) {a})");
                for i in 1..w {
                    acc = format!("(bvxor {acc} ((_ extract {i} {i}) {a}))");
                }
                acc
            }
        }
        ExprKind::Add(a, b) => bin("bvadd", m, *a, *b, step, cache, _defs),
        ExprKind::Sub(a, b) => bin("bvsub", m, *a, *b, step, cache, _defs),
        ExprKind::Mul(a, b) => bin("bvmul", m, *a, *b, step, cache, _defs),
        ExprKind::UDiv(a, b) => bin("bvudiv", m, *a, *b, step, cache, _defs),
        ExprKind::SDiv(a, b) => bin("bvsdiv", m, *a, *b, step, cache, _defs),
        ExprKind::UMod(a, b) => bin("bvurem", m, *a, *b, step, cache, _defs),
        ExprKind::SMod(a, b) => bin("bvsrem", m, *a, *b, step, cache, _defs),
        ExprKind::And(a, b) => bin("bvand", m, *a, *b, step, cache, _defs),
        ExprKind::Or(a, b) => bin("bvor", m, *a, *b, step, cache, _defs),
        ExprKind::Xor(a, b) => bin("bvxor", m, *a, *b, step, cache, _defs),
        ExprKind::Shl(a, b) => bin("bvshl", m, *a, *b, step, cache, _defs),
        ExprKind::Lshr(a, b) => bin("bvlshr", m, *a, *b, step, cache, _defs),
        ExprKind::Ashr(a, b) => bin("bvashr", m, *a, *b, step, cache, _defs),
        ExprKind::Eq(a, b) => cmp("=", m, *a, *b, step, cache, _defs),
        ExprKind::Ne(a, b) => {
            let a = emit_expr_dag(m, *a, step, cache, _defs);
            let b = emit_expr_dag(m, *b, step, cache, _defs);
            format!("(ite (= {a} {b}) #b0 #b1)")
        }
        ExprKind::Ult(a, b) => cmp("bvult", m, *a, *b, step, cache, _defs),
        ExprKind::Ule(a, b) => cmp("bvule", m, *a, *b, step, cache, _defs),
        ExprKind::Slt(a, b) => cmp("bvslt", m, *a, *b, step, cache, _defs),
        ExprKind::Sle(a, b) => cmp("bvsle", m, *a, *b, step, cache, _defs),
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let c = emit_expr_dag(m, *cond, step, cache, _defs);
            let t = emit_expr_dag(m, *then_val, step, cache, _defs);
            let e = emit_expr_dag(m, *else_val, step, cache, _defs);
            format!("(ite (= {c} #b1) {t} {e})")
        }
        ExprKind::Concat(parts) => {
            let mut acc = emit_expr_dag(m, parts[0], step, cache, _defs);
            for &p in &parts[1..] {
                let ps = emit_expr_dag(m, p, step, cache, _defs);
                acc = format!("(concat {acc} {ps})");
            }
            acc
        }
        ExprKind::Slice { expr, hi, lo } => {
            let e = emit_expr_dag(m, *expr, step, cache, _defs);
            format!("((_ extract {hi} {lo}) {e})")
        }
        ExprKind::Zext { expr, width } => {
            let extra = *width - m.width(*expr);
            let e = emit_expr_dag(m, *expr, step, cache, _defs);
            format!("((_ zero_extend {extra}) {e})")
        }
        ExprKind::Sext { expr, width } => {
            let extra = *width - m.width(*expr);
            let e = emit_expr_dag(m, *expr, step, cache, _defs);
            format!("((_ sign_extend {extra}) {e})")
        }
        ExprKind::MemRead { mem, addr } => {
            let a = emit_expr_dag(m, *addr, step, cache, _defs);
            format!("(select {} {a})", mem_name(*mem, step))
        }
    };
    cache.insert(id, res.clone());
    res
}

fn bin(
    op: &str,
    m: &Module,
    a: ExprId,
    b: ExprId,
    step: usize,
    cache: &mut HashMap<ExprId, String>,
    defs: &mut String,
) -> String {
    let a = emit_expr_dag(m, a, step, cache, defs);
    let b = emit_expr_dag(m, b, step, cache, defs);
    format!("({op} {a} {b})")
}

fn cmp(
    op: &str,
    m: &Module,
    a: ExprId,
    b: ExprId,
    step: usize,
    cache: &mut HashMap<ExprId, String>,
    defs: &mut String,
) -> String {
    let a = emit_expr_dag(m, a, step, cache, defs);
    let b = emit_expr_dag(m, b, step, cache, defs);
    format!("(ite ({op} {a} {b}) #b1 #b0)")
}
