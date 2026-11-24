use anyhow::Result;
use std::collections::{HashMap, HashSet};
use vf_ir::{ExprId, ExprKind, Module, PropertyKind, SignalId, SignalKind};
use vf_smt::smt2;
use vf_smt::solver::{SatResult, SmtSolver, TraceFrame};

#[derive(Debug, Clone)]
pub struct DeadEndConfig {
    pub depth: usize,
    pub solver: String,
}

impl Default for DeadEndConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            solver: "z3".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeadEndOutcome {
    /// Design has no `assume` constraints, so no assumption dead-end is possible.
    NoAssumptions,
    /// Assumptions are already unsatisfiable at cycle 0 (contradictory initial constraints).
    InitialUnsat,
    /// Found a reachable dead-end state at `step`: assumptions hold for `0..step`,
    /// but from the resulting state at `step`, NO input assignment can satisfy `assume`!
    DeadEndFound { step: usize, trace: Vec<TraceFrame> },
    /// No assumption dead-end reachable within `depth` cycles.
    Clean { depth: usize },
    /// Solver returned Unknown.
    Unknown { step: usize },
}

/// Questa PropCheck Over-Constraint / Dead-End State Detector:
/// Checks whether `assume` constraints can drive the design into a reachable dead-end state
/// from which NO valid next-cycle input assignment can satisfy the assumptions.
pub fn check_deadends(m: &Module, cfg: &DeadEndConfig) -> Result<DeadEndOutcome> {
    let assumes: Vec<usize> = m
        .properties
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PropertyKind::Assume)
        .map(|(i, _)| i)
        .collect();

    if assumes.is_empty() {
        return Ok(DeadEndOutcome::NoAssumptions);
    }

    // 1. Check cycle-0 satisfiability of assumptions.
    {
        let mut s0 = SmtSolver::spawn(&cfg.solver)?;
        s0.send(&smt2::header(m))?;
        s0.send(&smt2::encode_step(m, 0, true, None))?;
        s0.send(&smt2::encode_assumes(m, 0))?;
        if s0.check_sat()? == SatResult::Unsat {
            return Ok(DeadEndOutcome::InitialUnsat);
        }
    }

    // Identify combinational wires vs state/free inputs.
    let ff_sigs: HashSet<SignalId> = m.ffs.iter().map(|f| f.signal).collect();
    let comb_sigs: HashSet<SignalId> = m.comb.iter().map(|c| c.signal).collect();

    // Quantified free variables at step `t`: primary inputs (and `AnySeq`) that are NOT driven
    // by comb/FF and NOT `AnyConst` (which are fixed across cycles).
    let free_inputs: Vec<SignalId> = m
        .signals
        .iter()
        .filter(|s| {
            matches!(s.kind, SignalKind::Input | SignalKind::AnySeq)
                && !ff_sigs.contains(&s.id)
                && !comb_sigs.contains(&s.id)
        })
        .map(|s| s.id)
        .collect();

    // Unroll step-by-step using `(set-logic ALL)` so Z3 can decide BV `forall` directly!
    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send("(set-logic ALL)\n(set-option :produce-models true)\n")?;

    // Encode step 0 + assumes(0).
    solver.send(&smt2::encode_step(m, 0, true, None))?;
    solver.send(&smt2::encode_assumes(m, 0))?;

    for t in 1..=cfg.depth {
        // Unroll transition `(t-1) -> t` (WITHOUT asserting assumes(t) yet!).
        solver.send(&smt2::encode_step(m, t, false, Some(t - 1)))?;

        // Test if there exists a reachable state at step `t` where ALL input valuations fail `assumes(t)`.
        solver.push()?;
        let deadend_assert = encode_deadend_query(m, t, &assumes, &free_inputs);
        solver.send(&deadend_assert)?;

        match solver.check_sat()? {
            SatResult::Sat => {
                let trace = solver.extract_trace(m, t + 1)?;
                solver.pop()?;
                return Ok(DeadEndOutcome::DeadEndFound { step: t, trace });
            }
            SatResult::Unsat => {
                solver.pop()?;
            }
            SatResult::Unknown => {
                solver.pop()?;
                return Ok(DeadEndOutcome::Unknown { step: t });
            }
        }

        // No dead-end at step `t`; assert `assumes(t)` and advance to `t + 1`.
        solver.send(&smt2::encode_assumes(m, t))?;
    }

    Ok(DeadEndOutcome::Clean { depth: cfg.depth })
}

/// Build an SMT-LIB2 assertion that NO choice of `free_inputs` at `step` satisfies all `assumes`.
/// Uses linear-size nested `let` bindings for topologically sorted combinational wires inside `forall`.
fn encode_deadend_query(
    m: &Module,
    step: usize,
    assumes: &[usize],
    free_inputs: &[SignalId],
) -> String {
    let free_set: HashSet<SignalId> = free_inputs.iter().copied().collect();
    let comb_set: HashSet<SignalId> = m.comb.iter().map(|c| c.signal).collect();

    // Name resolver inside the quantified scope:
    // - free inputs -> `q_s<id>`
    // - comb wires  -> `qc_s<id>` (bound by nested `let` in topological order)
    // - FFs / AnyConst / everything else -> `s<id>_t<step>` (fixed by step `step` state!)
    let sig_ref = |sid: SignalId| -> String {
        if free_set.contains(&sid) {
            format!("q_s{}", sid.0)
        } else if comb_set.contains(&sid) {
            format!("qc_s{}", sid.0)
        } else {
            smt2::sig_name(sid, step)
        }
    };

    // Build body: `(not (and (=> (= en_0 #b1) (= cond_0 #b1)) ...))`
    let mut clauses = Vec::with_capacity(assumes.len());
    for &idx in assumes {
        let p = &m.properties[idx];
        let mut cache = HashMap::new();
        let en = emit_expr_scoped(m, p.en, step, &sig_ref, &mut cache);
        let cond = emit_expr_scoped(m, p.cond, step, &sig_ref, &mut cache);
        clauses.push(format!("(=> (= {en} #b1) (= {cond} #b1))"));
    }
    let all_assumes = if clauses.len() == 1 {
        clauses.pop().unwrap()
    } else {
        format!("(and {})", clauses.join(" "))
    };
    let mut body = format!("(not {all_assumes})");

    // Wrap with nested `let` bindings in reverse topological order so inner expressions see all comb wires!
    for ca in m.comb.iter().rev() {
        let mut cache = HashMap::new();
        let rhs = emit_expr_scoped(m, ca.value, step, &sig_ref, &mut cache);
        body = format!("(let ((qc_s{} {rhs})) {body})", ca.signal.0);
    }

    if free_inputs.is_empty() {
        format!("(assert {body})\n")
    } else {
        let binders: Vec<String> = free_inputs
            .iter()
            .map(|&sid| format!("(q_s{} (_ BitVec {}))", sid.0, m.signal(sid).width))
            .collect();
        format!("(assert (forall ({}) {body}))\n", binders.join(" "))
    }
}

fn emit_expr_scoped<F>(
    m: &Module,
    id: ExprId,
    step: usize,
    sig_ref: &F,
    cache: &mut HashMap<ExprId, String>,
) -> String
where
    F: Fn(SignalId) -> String,
{
    if let Some(s) = cache.get(&id) {
        return s.clone();
    }
    let res = match &m.expr(id).kind {
        ExprKind::Const(c) => smt2::smt_const(c),
        ExprKind::Signal(s) => sig_ref(*s),
        ExprKind::Not(a) => {
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
            format!("(bvnot {a})")
        }
        ExprKind::Neg(a) => {
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
            format!("(bvneg {a})")
        }
        ExprKind::RedAnd(a) => {
            let w = m.width(*a);
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
            let ones = "1".repeat(w as usize);
            format!("(ite (= {a} #b{ones}) #b1 #b0)")
        }
        ExprKind::RedOr(a) => {
            let w = m.width(*a);
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
            let zeros = "0".repeat(w as usize);
            format!("(ite (= {a} #b{zeros}) #b0 #b1)")
        }
        ExprKind::RedXor(a) => {
            let w = m.width(*a);
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
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
        ExprKind::Add(a, b) => sbin("bvadd", m, *a, *b, step, sig_ref, cache),
        ExprKind::Sub(a, b) => sbin("bvsub", m, *a, *b, step, sig_ref, cache),
        ExprKind::Mul(a, b) => sbin("bvmul", m, *a, *b, step, sig_ref, cache),
        ExprKind::UDiv(a, b) => sbin("bvudiv", m, *a, *b, step, sig_ref, cache),
        ExprKind::SDiv(a, b) => sbin("bvsdiv", m, *a, *b, step, sig_ref, cache),
        ExprKind::UMod(a, b) => sbin("bvurem", m, *a, *b, step, sig_ref, cache),
        ExprKind::SMod(a, b) => sbin("bvsrem", m, *a, *b, step, sig_ref, cache),
        ExprKind::And(a, b) => sbin("bvand", m, *a, *b, step, sig_ref, cache),
        ExprKind::Or(a, b) => sbin("bvor", m, *a, *b, step, sig_ref, cache),
        ExprKind::Xor(a, b) => sbin("bvxor", m, *a, *b, step, sig_ref, cache),
        ExprKind::Shl(a, b) => sbin("bvshl", m, *a, *b, step, sig_ref, cache),
        ExprKind::Lshr(a, b) => sbin("bvlshr", m, *a, *b, step, sig_ref, cache),
        ExprKind::Ashr(a, b) => sbin("bvashr", m, *a, *b, step, sig_ref, cache),
        ExprKind::Eq(a, b) => scmp("=", m, *a, *b, step, sig_ref, cache),
        ExprKind::Ne(a, b) => {
            let a = emit_expr_scoped(m, *a, step, sig_ref, cache);
            let b = emit_expr_scoped(m, *b, step, sig_ref, cache);
            format!("(ite (= {a} {b}) #b0 #b1)")
        }
        ExprKind::Ult(a, b) => scmp("bvult", m, *a, *b, step, sig_ref, cache),
        ExprKind::Ule(a, b) => scmp("bvule", m, *a, *b, step, sig_ref, cache),
        ExprKind::Slt(a, b) => scmp("bvslt", m, *a, *b, step, sig_ref, cache),
        ExprKind::Sle(a, b) => scmp("bvsle", m, *a, *b, step, sig_ref, cache),
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let c = emit_expr_scoped(m, *cond, step, sig_ref, cache);
            let t = emit_expr_scoped(m, *then_val, step, sig_ref, cache);
            let e = emit_expr_scoped(m, *else_val, step, sig_ref, cache);
            format!("(ite (= {c} #b1) {t} {e})")
        }
        ExprKind::Concat(parts) => {
            let mut acc = emit_expr_scoped(m, parts[0], step, sig_ref, cache);
            for &p in &parts[1..] {
                let ps = emit_expr_scoped(m, p, step, sig_ref, cache);
                acc = format!("(concat {acc} {ps})");
            }
            acc
        }
        ExprKind::Slice { expr, hi, lo } => {
            let e = emit_expr_scoped(m, *expr, step, sig_ref, cache);
            format!("((_ extract {hi} {lo}) {e})")
        }
        ExprKind::Zext { expr, width } => {
            let extra = *width - m.width(*expr);
            let e = emit_expr_scoped(m, *expr, step, sig_ref, cache);
            format!("((_ zero_extend {extra}) {e})")
        }
        ExprKind::Sext { expr, width } => {
            let extra = *width - m.width(*expr);
            let e = emit_expr_scoped(m, *expr, step, sig_ref, cache);
            format!("((_ sign_extend {extra}) {e})")
        }
        ExprKind::MemRead { mem, addr } => {
            let a = emit_expr_scoped(m, *addr, step, sig_ref, cache);
            format!("(select {} {a})", smt2::mem_name(*mem, step))
        }
    };
    cache.insert(id, res.clone());
    res
}

fn sbin<F: Fn(SignalId) -> String>(
    op: &str,
    m: &Module,
    a: ExprId,
    b: ExprId,
    step: usize,
    sig_ref: &F,
    cache: &mut HashMap<ExprId, String>,
) -> String {
    let a = emit_expr_scoped(m, a, step, sig_ref, cache);
    let b = emit_expr_scoped(m, b, step, sig_ref, cache);
    format!("({op} {a} {b})")
}

fn scmp<F: Fn(SignalId) -> String>(
    op: &str,
    m: &Module,
    a: ExprId,
    b: ExprId,
    step: usize,
    sig_ref: &F,
    cache: &mut HashMap<ExprId, String>,
) -> String {
    let a = emit_expr_scoped(m, a, step, sig_ref, cache);
    let b = emit_expr_scoped(m, b, step, sig_ref, cache);
    format!("(ite ({op} {a} {b}) #b1 #b0)")
}
