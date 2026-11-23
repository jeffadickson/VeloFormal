use crate::ast::{AssertionExpr, ElaboratedDesign, Expr, Stmt, Timing};
use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};
use vf_ir::{
    AsyncReset, CombAssign, ConstBits, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property,
    PropertyKind, SignalId, SignalKind,
};

struct Lowerer {
    module: Module,
    mem_lo: HashMap<MemId, i32>,
    init_state_sig: Option<SignalId>,
    past_counter: usize,
    any_counter: usize,
    prop_counter: usize,
    /// Pending flip-flops created by `$past` / `$rose` / `$fell` / `$stable` / `$changed` / `$initstate` / `|=>`.
    aux_ffs: Vec<Ff>,
    /// Initial values collected from variable declarations and `initial` blocks.
    inits: HashMap<SignalId, ConstBits>,
    /// Signals driven by flip-flops (sequential).
    seq_drivers: HashMap<SignalId, (ExprId, Option<AsyncReset>)>,
    /// Signals driven combinationally.
    comb_drivers: HashMap<SignalId, ExprId>,
}

impl Lowerer {
    fn new(name: &str) -> Self {
        Self {
            module: Module::new(name),
            mem_lo: HashMap::new(),
            init_state_sig: None,
            past_counter: 0,
            any_counter: 0,
            prop_counter: 0,
            aux_ffs: Vec::new(),
            inits: HashMap::new(),
            seq_drivers: HashMap::new(),
            comb_drivers: HashMap::new(),
        }
    }

    fn lower_design(mut self, design: &ElaboratedDesign) -> Result<Module> {
        // 1. Register all signals and memories.
        for s in &design.signals {
            let kind = match s.direction.as_str() {
                "input" => SignalKind::Input,
                "output" => SignalKind::Output,
                _ => SignalKind::Wire,
            };
            let id = self.module.add_signal(&s.name, s.width, s.signed, kind);
            if let Some(init_expr) = &s.init {
                if let Some(c) = self.try_eval_const(init_expr, s.width, s.signed) {
                    self.inits.insert(id, c);
                }
            }
        }

        for m in &design.memories {
            let addr_width = if m.depth <= 1 {
                1
            } else {
                (m.depth - 1).ilog2() + 1
            };
            let id = self
                .module
                .add_memory(&m.name, addr_width, m.data_width, m.depth);
            self.mem_lo.insert(id, m.lo_index);
        }

        // 2. Lower continuous assignments (`assign` and flattened sub-instance ports).
        let empty_env = HashMap::new();
        for ca in &design.assigns {
            let rhs = self.lower_expr(&ca.rhs, &empty_env)?;
            self.lower_comb_lhs(&ca.lhs, rhs)?;
        }

        // 3. Lower procedural blocks (`initial`, `always_comb`, `always_ff`, `always`).
        for proc in &design.processes {
            match proc.kind.as_str() {
                "initial" => self.lower_initial(&proc.body)?,
                "always_comb" | "always_latch" => self.lower_comb_proc(&proc.body)?,
                "always_ff" => self.lower_seq_proc(&proc.body)?,
                "always" => self.lower_always(&proc.body)?,
                _ => {}
            }
        }

        // 4. Finalize signal kinds, FFs, combinational assigns, and init values.
        for (&sig, init) in &self.inits {
            self.module.signal_mut(sig).init = Some(init.clone());
        }

        for (sig, (next, async_reset)) in std::mem::take(&mut self.seq_drivers) {
            let s = self.module.signal_mut(sig);
            if s.kind == SignalKind::Wire {
                s.kind = SignalKind::Reg;
            }
            let init = self.inits.get(&sig).cloned();
            self.module.ffs.push(Ff {
                signal: sig,
                next,
                init,
                async_reset,
            });
        }

        for ff in std::mem::take(&mut self.aux_ffs) {
            self.module.ffs.push(ff);
        }

        let ff_sigs: HashSet<SignalId> = self.module.ffs.iter().map(|f| f.signal).collect();
        for (sig, val) in std::mem::take(&mut self.comb_drivers) {
            if !ff_sigs.contains(&sig) {
                self.module.comb.push(CombAssign {
                    signal: sig,
                    value: val,
                });
            }
        }

        vf_ir::opt::resolve_self_slices(&mut self.module);
        self.module
            .topo_sort_comb()
            .map_err(|e| anyhow!(e))?;
        vf_ir::opt::optimize(&mut self.module);
        Ok(self.module)
    }

    fn lower_comb_lhs(&mut self, lhs: &Expr, rhs: ExprId) -> Result<()> {
        match lhs {
            Expr::Ref { name, width, .. } => {
                let sig = self
                    .module
                    .find_signal(name)
                    .ok_or_else(|| anyhow!("unknown signal in LHS: {name}"))?;
                let val = self.module.resize(rhs, *width, false);
                self.comb_drivers.insert(sig, val);
                Ok(())
            }
            Expr::Concat { operands, .. } => {
                let mut offset = self.module.width(rhs);
                for op in operands {
                    let w = op.width();
                    offset -= w;
                    let slice = self.module.slice(rhs, offset + w - 1, offset);
                    self.lower_comb_lhs(op, slice)?;
                }
                Ok(())
            }
            Expr::BitSelect {
                value,
                selector,
                range_left,
                range_right,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        if let Some(idx) = self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64()) {
                            let bit = self.normalize_bit_index(idx as i64, *range_left, *range_right);
                            let base = self
                                .comb_drivers
                                .get(&sig)
                                .copied()
                                .unwrap_or_else(|| self.module.c_u64(self.module.signal(sig).width, 0));
                            let updated = self.module.update_slice(base, bit, bit, rhs);
                            self.comb_drivers.insert(sig, updated);
                            return Ok(());
                        }
                    }
                }
                bail!("unsupported dynamic bit-select in continuous assignment LHS")
            }
            Expr::RangeSelect {
                value,
                left_const: Some(l),
                right_const: Some(r),
                range_left,
                range_right,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        let hi = self.normalize_bit_index(*l, *range_left, *range_right);
                        let lo = self.normalize_bit_index(*r, *range_left, *range_right);
                        let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
                        let base = self
                            .comb_drivers
                            .get(&sig)
                            .copied()
                            .unwrap_or_else(|| self.module.c_u64(self.module.signal(sig).width, 0));
                        let updated = self.module.update_slice(base, hi, lo, rhs);
                        self.comb_drivers.insert(sig, updated);
                        return Ok(());
                    }
                }
                bail!("unsupported range-select in continuous assignment LHS")
            }
            _ => bail!("unsupported continuous assignment LHS: {lhs:?}"),
        }
    }

    fn lower_initial(&mut self, stmt: &Stmt) -> Result<()> {
        match stmt {
            Stmt::Empty => Ok(()),
            Stmt::Block { items, .. } => {
                for item in items {
                    self.lower_initial(item)?;
                }
                Ok(())
            }
            Stmt::Assign { lhs, rhs } | Stmt::Nba { lhs, rhs } => {
                match lhs {
                    Expr::Ref { name, width, signed } => {
                        if let Some(sig) = self.module.find_signal(name) {
                            if let Some(c) = self.try_eval_const(rhs, *width, *signed) {
                                self.inits.insert(sig, c);
                            }
                        }
                    }
                    Expr::MemSelect { value, selector, .. } => {
                        if let Expr::Ref { name, .. } = value.as_ref() {
                            if let Some(&mem_id) = self.module.mem_by_name.get(name) {
                                let dw = self.module.memory(mem_id).data_width;
                                if let (Some(addr), Some(val)) = (
                                    self.try_eval_const(selector, 32, false).and_then(|c| c.to_u64()),
                                    self.try_eval_const(rhs, dw, false),
                                ) {
                                    let lo = self.mem_lo.get(&mem_id).copied().unwrap_or(0);
                                    let norm = (addr as i64 - lo as i64) as u32;
                                    self.module.memory_mut(mem_id).init.push((norm, val));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            Stmt::ImmediateAssert {
                assert_kind,
                cond,
                loc,
            } => {
                // `initial assume(...)` or `initial assert(...)` is guarded by `$initstate`.
                let init_en = self.get_or_create_initstate();
                let env = HashMap::new();
                let c = self.lower_expr(cond, &env)?;
                let c = self.module.to_bool(c);
                self.add_property(assert_kind, c, init_en, loc);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn lower_always(&mut self, stmt: &Stmt) -> Result<()> {
        if let Stmt::Timed { timing, body } = stmt {
            match timing {
                Timing::Implicit => self.lower_comb_proc(body),
                Timing::SignalEvent { edge, expr } => {
                    if edge == "none" {
                        self.lower_comb_proc(body)
                    } else {
                        self.mark_clock_expr(expr);
                        self.lower_clocked_body(body, None)
                    }
                }
                Timing::EventList { events } => {
                    let has_edge = events.iter().any(|e| {
                        matches!(e, Timing::SignalEvent { edge, .. } if edge != "none")
                    });
                    if !has_edge {
                        self.lower_comb_proc(body)
                    } else {
                        let async_rst = self.extract_async_reset_from_events(events);
                        self.lower_clocked_body(body, async_rst)
                    }
                }
                Timing::Other => self.lower_comb_proc(body),
            }
        } else {
            self.lower_comb_proc(stmt)
        }
    }

    fn lower_seq_proc(&mut self, stmt: &Stmt) -> Result<()> {
        if let Stmt::Timed { timing, body } = stmt {
            let async_rst = match timing {
                Timing::SignalEvent { expr, .. } => {
                    self.mark_clock_expr(expr);
                    None
                }
                Timing::EventList { events } => self.extract_async_reset_from_events(events),
                _ => None,
            };
            self.lower_clocked_body(body, async_rst)
        } else {
            self.lower_clocked_body(stmt, None)
        }
    }

    fn mark_clock_expr(&mut self, expr: &Expr) {
        if let Expr::Ref { name, .. } = expr {
            if let Some(sig) = self.module.find_signal(name) {
                self.module.signal_mut(sig).is_clock = true;
            }
        }
    }

    fn extract_async_reset_from_events(&mut self, events: &[Timing]) -> Option<(SignalId, bool)> {
        let mut async_rst = None;
        for (idx, ev) in events.iter().enumerate() {
            if let Timing::SignalEvent { edge, expr } = ev {
                if idx == 0 {
                    self.mark_clock_expr(expr);
                } else if let Expr::Ref { name, .. } = expr {
                    if let Some(sig) = self.module.find_signal(name) {
                        let active_low = edge == "negedge";
                        async_rst = Some((sig, active_low));
                    }
                }
            }
        }
        async_rst
    }

    fn lower_comb_proc(&mut self, stmt: &Stmt) -> Result<()> {
        let mut env: HashMap<SignalId, ExprId> = HashMap::new();
        let mut nba_writes: HashMap<SignalId, ExprId> = HashMap::new();
        let mut mem_writes: Vec<MemWrite> = Vec::new();
        let en = self.module.c_bool(true);
        self.lower_stmt(stmt, en, &mut env, &mut nba_writes, &mut mem_writes, false)?;
        for (sig, val) in env {
            self.comb_drivers.insert(sig, val);
        }
        Ok(())
    }

    fn lower_clocked_body(
        &mut self,
        stmt: &Stmt,
        async_rst: Option<(SignalId, bool)>,
    ) -> Result<()> {
        let mut blocking_env: HashMap<SignalId, ExprId> = HashMap::new();
        let mut nba_writes: HashMap<SignalId, ExprId> = HashMap::new();
        let mut mem_writes: Vec<MemWrite> = Vec::new();
        let en = self.module.c_bool(true);
        self.lower_stmt(
            stmt,
            en,
            &mut blocking_env,
            &mut nba_writes,
            &mut mem_writes,
            true,
        )?;

        // Async reset metadata (synchronous-semantics mux already lowered in `if (rst)` body,
        // and we also record async_reset if recognized for BTOR2/SMT2 inspection).
        let _ = async_rst;

        // Blocking assignments inside clocked blocks that aren't purely temporary also update regs.
        for (sig, val) in nba_writes {
            self.seq_drivers.insert(sig, (val, None));
        }
        for (sig, val) in blocking_env {
            self.seq_drivers.entry(sig).or_insert((val, None));
        }
        self.module.mem_writes.extend(mem_writes);
        Ok(())
    }

    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        guard: ExprId,
        env: &mut HashMap<SignalId, ExprId>,
        nba: &mut HashMap<SignalId, ExprId>,
        mem_writes: &mut Vec<MemWrite>,
        is_clocked: bool,
    ) -> Result<()> {
        match stmt {
            Stmt::Empty => Ok(()),
            Stmt::Block { items, .. } => {
                for item in items {
                    self.lower_stmt(item, guard, env, nba, mem_writes, is_clocked)?;
                }
                Ok(())
            }
            Stmt::Timed { body, .. } => {
                self.lower_stmt(body, guard, env, nba, mem_writes, is_clocked)
            }
            Stmt::Assign { lhs, rhs } => {
                let val = self.lower_expr(rhs, env)?;
                self.apply_write(lhs, val, guard, env, mem_writes, false)
            }
            Stmt::Nba { lhs, rhs } => {
                let val = self.lower_expr(rhs, env)?;
                self.apply_write(lhs, val, guard, nba, mem_writes, true)
            }
            Stmt::If {
                cond,
                then,
                else_branch,
            } => {
                let cond_raw = self.lower_expr(cond, env)?;
                let cond_b = self.module.to_bool(cond_raw);
                let not_cond = self.module.logic_not(cond_b);
                let then_guard = self.module.logic_and(guard, cond_b);
                let else_guard = self.module.logic_and(guard, not_cond);

                let mut then_env = env.clone();
                let mut then_nba = nba.clone();
                self.lower_stmt(
                    then,
                    then_guard,
                    &mut then_env,
                    &mut then_nba,
                    mem_writes,
                    is_clocked,
                )?;

                let mut else_env = env.clone();
                let mut else_nba = nba.clone();
                if let Some(eb) = else_branch {
                    self.lower_stmt(
                        eb,
                        else_guard,
                        &mut else_env,
                        &mut else_nba,
                        mem_writes,
                        is_clocked,
                    )?;
                }

                self.merge_maps(cond_b, env, &then_env, &else_env);
                self.merge_maps(cond_b, nba, &then_nba, &else_nba);
                Ok(())
            }
            Stmt::Case {
                expr,
                items,
                default,
            } => {
                let sel = self.lower_expr(expr, env)?;
                let sel_signed = expr.is_signed();

                // Lower case as priority if-else chain from top to bottom (matching SV semantics).
                let base_env = env.clone();
                let base_nba = nba.clone();

                let mut branch_results: Vec<(ExprId, HashMap<SignalId, ExprId>, HashMap<SignalId, ExprId>)> =
                    Vec::new();
                let mut unmatched = guard;

                for item in items {
                    let mut item_match = self.module.c_bool(false);
                    for e in &item.exprs {
                        let ev = self.lower_expr(e, &base_env)?;
                        let eq = self.module.eq(sel, ev, sel_signed);
                        item_match = self.module.logic_or(item_match, eq);
                    }
                    let branch_guard = self.module.logic_and(unmatched, item_match);
                    let not_match = self.module.logic_not(item_match);
                    unmatched = self.module.logic_and(unmatched, not_match);

                    let mut b_env = base_env.clone();
                    let mut b_nba = base_nba.clone();
                    self.lower_stmt(
                        &item.body,
                        branch_guard,
                        &mut b_env,
                        &mut b_nba,
                        mem_writes,
                        is_clocked,
                    )?;
                    branch_results.push((item_match, b_env, b_nba));
                }

                let mut cur_env = base_env.clone();
                let mut cur_nba = base_nba.clone();
                if let Some(def) = default {
                    self.lower_stmt(
                        def,
                        unmatched,
                        &mut cur_env,
                        &mut cur_nba,
                        mem_writes,
                        is_clocked,
                    )?;
                }

                for (cond, b_env, b_nba) in branch_results.into_iter().rev() {
                    let prev_env = cur_env.clone();
                    let prev_nba = cur_nba.clone();
                    self.merge_maps(cond, &mut cur_env, &b_env, &prev_env);
                    self.merge_maps(cond, &mut cur_nba, &b_nba, &prev_nba);
                }

                *env = cur_env;
                *nba = cur_nba;
                Ok(())
            }
            Stmt::ImmediateAssert {
                assert_kind,
                cond,
                loc,
            } => {
                let c = self.lower_expr(cond, env)?;
                let c = self.module.to_bool(c);
                self.add_property(assert_kind, c, guard, loc);
                Ok(())
            }
            Stmt::ConcurrentAssert {
                assert_kind,
                prop,
                loc,
            } => {
                self.lower_concurrent_assert(assert_kind, prop, guard, env, loc)?;
                Ok(())
            }
            Stmt::ExprStmt { .. } => Ok(()),
            Stmt::ForLoop { loc } => {
                bail!("procedural for-loop at {loc} is not yet lowered (use generate-for or submit issue)")
            }
            Stmt::Unsupported { loc } => {
                bail!("unsupported SystemVerilog statement at {loc}")
            }
        }
    }

    fn lower_concurrent_assert(
        &mut self,
        assert_kind: &str,
        prop: &AssertionExpr,
        guard: ExprId,
        env: &HashMap<SignalId, ExprId>,
        loc: &str,
    ) -> Result<()> {
        match prop {
            AssertionExpr::Clocking { clocking, expr } => {
                if let Timing::SignalEvent { expr: clk, .. } = clocking {
                    self.mark_clock_expr(clk);
                }
                self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
            }
            AssertionExpr::DisableIff { cond, expr } => {
                let dis = self.lower_expr(cond, env)?;
                let not_dis = self.module.logic_not(dis);
                let new_guard = self.module.logic_and(guard, not_dis);
                self.lower_concurrent_assert(assert_kind, expr, new_guard, env, loc)
            }
            AssertionExpr::Simple { expr } => {
                let c = self.lower_expr(expr, env)?;
                let c = self.module.to_bool(c);
                self.add_property(assert_kind, c, guard, loc);
                Ok(())
            }
            AssertionExpr::Unary { op, expr } if op == "Not" => {
                let inner = self.lower_assertion_bool(expr, env)?;
                let negated = self.module.logic_not(inner);
                self.add_property(assert_kind, negated, guard, loc);
                Ok(())
            }
            AssertionExpr::Binary { op, left, right } => {
                match op.as_str() {
                    "OverlappedImplication" | "Implies" => {
                        // `A |-> B` is `A => B` in the same cycle.
                        let a = self.lower_assertion_bool(left, env)?;
                        let b = self.lower_assertion_bool(right, env)?;
                        let new_guard = self.module.logic_and(guard, a);
                        self.add_property(assert_kind, b, new_guard, loc);
                        Ok(())
                    }
                    "NonOverlappedImplication" => {
                        // `A |=> B` means: if `guard && A` held in the previous cycle, `B` holds now!
                        let a = self.lower_assertion_bool(left, env)?;
                        let trig = self.module.logic_and(guard, a);
                        let prev_trig = self.make_past_reg(trig, Some(ConstBits::zero(1)));
                        let b = self.lower_assertion_bool(right, env)?;
                        self.add_property(assert_kind, b, prev_trig, loc);
                        Ok(())
                    }
                    "And" => {
                        let a = self.lower_assertion_bool(left, env)?;
                        let b = self.lower_assertion_bool(right, env)?;
                        let both = self.module.logic_and(a, b);
                        self.add_property(assert_kind, both, guard, loc);
                        Ok(())
                    }
                    "Or" => {
                        let a = self.lower_assertion_bool(left, env)?;
                        let b = self.lower_assertion_bool(right, env)?;
                        let either = self.module.logic_or(a, b);
                        self.add_property(assert_kind, either, guard, loc);
                        Ok(())
                    }
                    other => bail!("unsupported SVA binary operator '{other}' at {loc}"),
                }
            }
            _ => bail!("unsupported concurrent assertion expression at {loc}"),
        }
    }

    fn lower_assertion_bool(
        &mut self,
        prop: &AssertionExpr,
        env: &HashMap<SignalId, ExprId>,
    ) -> Result<ExprId> {
        match prop {
            AssertionExpr::Simple { expr } => {
                let c = self.lower_expr(expr, env)?;
                Ok(self.module.to_bool(c))
            }
            AssertionExpr::Unary { op, expr } if op == "Not" => {
                let c = self.lower_assertion_bool(expr, env)?;
                Ok(self.module.logic_not(c))
            }
            AssertionExpr::Binary { op, left, right } if op == "And" => {
                let a = self.lower_assertion_bool(left, env)?;
                let b = self.lower_assertion_bool(right, env)?;
                Ok(self.module.logic_and(a, b))
            }
            AssertionExpr::Binary { op, left, right } if op == "Or" => {
                let a = self.lower_assertion_bool(left, env)?;
                let b = self.lower_assertion_bool(right, env)?;
                Ok(self.module.logic_or(a, b))
            }
            _ => bail!("nested temporal SVA expression not yet supported"),
        }
    }

    fn add_property(&mut self, kind_str: &str, cond: ExprId, en: ExprId, loc: &str) {
        let kind = match kind_str {
            "assume" => PropertyKind::Assume,
            "cover" => PropertyKind::Cover,
            _ => PropertyKind::Assert,
        };
        let idx = self.prop_counter;
        self.prop_counter += 1;
        let name = if loc.is_empty() {
            format!("{kind_str}_{idx}")
        } else {
            format!("{kind_str}_{idx}@{loc}")
        };
        self.module.properties.push(Property {
            name,
            kind,
            cond,
            en,
            source: if loc.is_empty() {
                None
            } else {
                Some(loc.to_string())
            },
        });
    }

    fn merge_maps(
        &mut self,
        cond: ExprId,
        target: &mut HashMap<SignalId, ExprId>,
        then_map: &HashMap<SignalId, ExprId>,
        else_map: &HashMap<SignalId, ExprId>,
    ) {
        let mut keys: Vec<SignalId> = then_map
            .keys()
            .chain(else_map.keys())
            .copied()
            .collect();
        keys.sort_by_key(|k| k.0);
        keys.dedup();

        for sig in keys {
            let t = then_map
                .get(&sig)
                .copied()
                .or_else(|| target.get(&sig).copied())
                .unwrap_or_else(|| self.module.sig_expr(sig));
            let e = else_map
                .get(&sig)
                .copied()
                .or_else(|| target.get(&sig).copied())
                .unwrap_or_else(|| self.module.sig_expr(sig));
            let merged = self.module.mux(cond, t, e);
            target.insert(sig, merged);
        }
    }

    fn apply_write(
        &mut self,
        lhs: &Expr,
        rhs: ExprId,
        guard: ExprId,
        target: &mut HashMap<SignalId, ExprId>,
        mem_writes: &mut Vec<MemWrite>,
        is_nba: bool,
    ) -> Result<()> {
        match lhs {
            Expr::Ref { name, width, .. } => {
                let sig = self
                    .module
                    .find_signal(name)
                    .ok_or_else(|| anyhow!("unknown signal in assignment LHS: {name}"))?;
                let val = self.module.resize(rhs, *width, false);
                target.insert(sig, val);
                Ok(())
            }
            Expr::Concat { operands, .. } => {
                let mut offset = self.module.width(rhs);
                for op in operands {
                    let w = op.width();
                    offset -= w;
                    let slice = self.module.slice(rhs, offset + w - 1, offset);
                    self.apply_write(op, slice, guard, target, mem_writes, is_nba)?;
                }
                Ok(())
            }
            Expr::BitSelect {
                value,
                selector,
                range_left,
                range_right,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        let base = target
                            .get(&sig)
                            .copied()
                            .unwrap_or_else(|| self.module.sig_expr(sig));
                        if let Some(idx) = self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64()) {
                            let bit = self.normalize_bit_index(idx as i64, *range_left, *range_right);
                            let updated = self.module.update_slice(base, bit, bit, rhs);
                            target.insert(sig, updated);
                            return Ok(());
                        } else {
                            // Dynamic bit select write: update bit via shift mask.
                            let w = self.module.signal(sig).width;
                            let sel_e = self.lower_expr(selector, target)?;
                            let sel_w = self.module.resize(sel_e, w, false);
                            let one = self.module.c_u64(w, 1);
                            let mask = self.module.intern(w, ExprKind::Shl(one, sel_w));
                            let not_mask = self.module.not(mask);
                            let cleared = self.module.intern(w, ExprKind::And(base, not_mask));
                            let rhs_b = self.module.to_bool(rhs);
                            let bit_val = self.module.resize(rhs_b, w, false);
                            let shifted = self.module.intern(w, ExprKind::Shl(bit_val, sel_w));
                            let updated = self.module.intern(w, ExprKind::Or(cleared, shifted));
                            target.insert(sig, updated);
                            return Ok(());
                        }
                    }
                }
                bail!("unsupported bit-select assignment LHS")
            }
            Expr::RangeSelect {
                value,
                left_const: Some(l),
                right_const: Some(r),
                range_left,
                range_right,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        let base = target
                            .get(&sig)
                            .copied()
                            .unwrap_or_else(|| self.module.sig_expr(sig));
                        let hi = self.normalize_bit_index(*l, *range_left, *range_right);
                        let lo = self.normalize_bit_index(*r, *range_left, *range_right);
                        let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
                        let updated = self.module.update_slice(base, hi, lo, rhs);
                        target.insert(sig, updated);
                        return Ok(());
                    }
                }
                bail!("unsupported range-select assignment LHS")
            }
            Expr::MemSelect {
                value, selector, ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(&mem_id) = self.module.mem_by_name.get(name) {
                        let mem = self.module.memory(mem_id);
                        let aw = mem.addr_width;
                        let dw = mem.data_width;
                        let addr_raw = self.lower_expr(selector, target)?;
                        let addr = self.module.resize(addr_raw, aw, false);
                        let data = self.module.resize(rhs, dw, false);
                        mem_writes.push(MemWrite {
                            mem: mem_id,
                            addr,
                            data,
                            enable: guard,
                        });
                        return Ok(());
                    }
                }
                bail!("unknown memory in assignment LHS")
            }
            _ => bail!("unsupported assignment LHS: {lhs:?}"),
        }
    }

    fn normalize_bit_index(&self, idx: i64, range_left: Option<i64>, range_right: Option<i64>) -> u32 {
        match (range_left, range_right) {
            (Some(l), Some(r)) if l >= r => (idx - r) as u32,
            (Some(_l), Some(r)) => (r - idx) as u32,
            _ => idx as u32,
        }
    }

    fn get_or_create_initstate(&mut self) -> ExprId {
        if let Some(sig) = self.init_state_sig {
            return self.module.sig_expr(sig);
        }
        let sig = self
            .module
            .add_signal("$initstate", 1, false, SignalKind::Reg);
        let one = ConstBits::one(1);
        let zero = self.module.c_bool(false);
        self.module.signal_mut(sig).init = Some(one.clone());
        self.aux_ffs.push(Ff {
            signal: sig,
            next: zero,
            init: Some(one),
            async_reset: None,
        });
        self.init_state_sig = Some(sig);
        self.module.sig_expr(sig)
    }

    fn make_past_reg(&mut self, expr: ExprId, init: Option<ConstBits>) -> ExprId {
        let w = self.module.width(expr);
        let name = format!("$past_{}", self.past_counter);
        self.past_counter += 1;
        let sig = self.module.add_signal(name, w, false, SignalKind::Reg);
        if let Some(c) = &init {
            self.module.signal_mut(sig).init = Some(c.clone());
        }
        self.aux_ffs.push(Ff {
            signal: sig,
            next: expr,
            init,
            async_reset: None,
        });
        self.module.sig_expr(sig)
    }

    fn lower_expr(&mut self, expr: &Expr, env: &HashMap<SignalId, ExprId>) -> Result<ExprId> {
        match expr {
            Expr::Const { width, bits, .. } => {
                Ok(self.module.c_bits(ConstBits::new(*width, bits.clone())))
            }
            Expr::Ref { name, .. } => {
                let sig = self
                    .module
                    .find_signal(name)
                    .ok_or_else(|| anyhow!("unknown signal reference: '{name}'"))?;
                if let Some(&val) = env.get(&sig) {
                    Ok(val)
                } else {
                    Ok(self.module.sig_expr(sig))
                }
            }
            Expr::Conv {
                width,
                signed: _,
                operand,
            } => {
                let op_signed = operand.is_signed();
                let v = self.lower_expr(operand, env)?;
                Ok(self.module.resize(v, *width, op_signed))
            }
            Expr::Unary {
                op,
                width,
                operand,
                ..
            } => {
                let v = self.lower_expr(operand, env)?;
                let op_signed = operand.is_signed();
                match op.as_str() {
                    "Plus" => Ok(self.module.resize(v, *width, op_signed)),
                    "Minus" => {
                        let r = self.module.resize(v, *width, op_signed);
                        Ok(self.module.intern(*width, ExprKind::Neg(r)))
                    }
                    "BitwiseNot" => {
                        let r = self.module.resize(v, *width, op_signed);
                        Ok(self.module.not(r))
                    }
                    "LogicalNot" => Ok(self.module.logic_not(v)),
                    "BitwiseAnd" => Ok(self.module.intern(1, ExprKind::RedAnd(v))),
                    "BitwiseNand" => {
                        let r = self.module.intern(1, ExprKind::RedAnd(v));
                        Ok(self.module.not(r))
                    }
                    "BitwiseOr" => Ok(self.module.intern(1, ExprKind::RedOr(v))),
                    "BitwiseNor" => {
                        let r = self.module.intern(1, ExprKind::RedOr(v));
                        Ok(self.module.not(r))
                    }
                    "BitwiseXor" => Ok(self.module.intern(1, ExprKind::RedXor(v))),
                    "BitwiseXnor" => {
                        let r = self.module.intern(1, ExprKind::RedXor(v));
                        Ok(self.module.not(r))
                    }
                    other => bail!("unsupported unary operator '{other}'"),
                }
            }
            Expr::Binary {
                op,
                width,
                signed,
                left,
                right,
            } => {
                let l = self.lower_expr(left, env)?;
                let r = self.lower_expr(right, env)?;
                let l_signed = left.is_signed();
                let r_signed = right.is_signed();
                let both_signed = l_signed && r_signed;
                match op.as_str() {
                    "LogicalAnd" => Ok(self.module.logic_and(l, r)),
                    "LogicalOr" => Ok(self.module.logic_or(l, r)),
                    "LogicalImplication" => {
                        let nl = self.module.logic_not(l);
                        Ok(self.module.logic_or(nl, r))
                    }
                    "LogicalEquivalence" => {
                        let lb = self.module.to_bool(l);
                        let rb = self.module.to_bool(r);
                        Ok(self.module.intern(1, ExprKind::Eq(lb, rb)))
                    }
                    "Equality" | "CaseEquality" | "WildcardEquality" => {
                        Ok(self.module.eq(l, r, both_signed))
                    }
                    "Inequality" | "CaseInequality" | "WildcardInequality" => {
                        let eq = self.module.eq(l, r, both_signed);
                        Ok(self.module.not(eq))
                    }
                    "LessThan" | "LessThanEqual" | "GreaterThan" | "GreaterThanEqual" => {
                        let w = self.module.width(l).max(self.module.width(r));
                        let l = self.module.resize(l, w, both_signed);
                        let r = self.module.resize(r, w, both_signed);
                        let kind = match (op.as_str(), both_signed) {
                            ("LessThan", false) => ExprKind::Ult(l, r),
                            ("LessThan", true) => ExprKind::Slt(l, r),
                            ("LessThanEqual", false) => ExprKind::Ule(l, r),
                            ("LessThanEqual", true) => ExprKind::Sle(l, r),
                            ("GreaterThan", false) => ExprKind::Ult(r, l),
                            ("GreaterThan", true) => ExprKind::Slt(r, l),
                            ("GreaterThanEqual", false) => ExprKind::Ule(r, l),
                            ("GreaterThanEqual", true) => ExprKind::Sle(r, l),
                            _ => unreachable!(),
                        };
                        Ok(self.module.intern(1, kind))
                    }
                    "Add" | "Subtract" | "Multiply" | "Divide" | "Mod" | "BinaryAnd"
                    | "BinaryOr" | "BinaryXor" | "BinaryXnor" => {
                        let l = self.module.resize(l, *width, l_signed);
                        let r = self.module.resize(r, *width, r_signed);
                        let res = match op.as_str() {
                            "Add" => self.module.intern(*width, ExprKind::Add(l, r)),
                            "Subtract" => self.module.intern(*width, ExprKind::Sub(l, r)),
                            "Multiply" => self.module.intern(*width, ExprKind::Mul(l, r)),
                            "Divide" => self.module.intern(
                                *width,
                                if *signed {
                                    ExprKind::SDiv(l, r)
                                } else {
                                    ExprKind::UDiv(l, r)
                                },
                            ),
                            "Mod" => self.module.intern(
                                *width,
                                if *signed {
                                    ExprKind::SMod(l, r)
                                } else {
                                    ExprKind::UMod(l, r)
                                },
                            ),
                            "BinaryAnd" => self.module.intern(*width, ExprKind::And(l, r)),
                            "BinaryOr" => self.module.intern(*width, ExprKind::Or(l, r)),
                            "BinaryXor" => self.module.intern(*width, ExprKind::Xor(l, r)),
                            "BinaryXnor" => {
                                let x = self.module.intern(*width, ExprKind::Xor(l, r));
                                self.module.not(x)
                            }
                            _ => unreachable!(),
                        };
                        Ok(res)
                    }
                    "LogicalShiftLeft" | "ArithmeticShiftLeft" => {
                        let l = self.module.resize(l, *width, l_signed);
                        let r = self.module.resize(r, *width, false);
                        Ok(self.module.intern(*width, ExprKind::Shl(l, r)))
                    }
                    "LogicalShiftRight" => {
                        let l = self.module.resize(l, *width, l_signed);
                        let r = self.module.resize(r, *width, false);
                        Ok(self.module.intern(*width, ExprKind::Lshr(l, r)))
                    }
                    "ArithmeticShiftRight" => {
                        let l = self.module.resize(l, *width, l_signed);
                        let r = self.module.resize(r, *width, false);
                        let kind = if *signed || l_signed {
                            ExprKind::Ashr(l, r)
                        } else {
                            ExprKind::Lshr(l, r)
                        };
                        Ok(self.module.intern(*width, kind))
                    }
                    other => bail!("unsupported binary operator '{other}'"),
                }
            }
            Expr::Ternary {
                width,
                cond,
                then,
                else_expr,
                ..
            } => {
                let c = self.lower_expr(cond, env)?;
                let t = self.lower_expr(then, env)?;
                let e = self.lower_expr(else_expr, env)?;
                let t = self.module.resize(t, *width, then.is_signed());
                let e = self.module.resize(e, *width, else_expr.is_signed());
                Ok(self.module.mux(c, t, e))
            }
            Expr::Concat { operands, .. } => {
                let mut parts = Vec::with_capacity(operands.len());
                for op in operands {
                    parts.push(self.lower_expr(op, env)?);
                }
                Ok(self.module.concat(parts))
            }
            Expr::Replicate { count, concat, .. } => {
                let inner = self.lower_expr(concat, env)?;
                let parts = vec![inner; *count as usize];
                Ok(self.module.concat(parts))
            }
            Expr::BitSelect {
                value,
                selector,
                range_left,
                range_right,
                ..
            } => {
                let base = self.lower_expr(value, env)?;
                if let Some(idx) = self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64()) {
                    let bit = self.normalize_bit_index(idx as i64, *range_left, *range_right);
                    Ok(self.module.slice(base, bit, bit))
                } else {
                    let w = self.module.width(base);
                    let sel = self.lower_expr(selector, env)?;
                    let sel = self.module.resize(sel, w, false);
                    let shifted = self.module.intern(w, ExprKind::Lshr(base, sel));
                    Ok(self.module.slice(shifted, 0, 0))
                }
            }
            Expr::MemSelect {
                value, selector, ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(&mem_id) = self.module.mem_by_name.get(name) {
                        let mem = self.module.memory(mem_id);
                        let aw = mem.addr_width;
                        let dw = mem.data_width;
                        let addr_raw = self.lower_expr(selector, env)?;
                        let addr = self.module.resize(addr_raw, aw, false);
                        return Ok(self.module.intern(dw, ExprKind::MemRead { mem: mem_id, addr }));
                    }
                }
                bail!("unknown memory in MemSelect")
            }
            Expr::RangeSelect {
                width,
                value,
                sel_kind,
                left_const,
                right_const,
                left,
                range_left,
                range_right,
                ..
            } => {
                let base = self.lower_expr(value, env)?;
                match (sel_kind.as_str(), left_const, right_const) {
                    ("simple", Some(l), Some(r)) => {
                        let hi = self.normalize_bit_index(*l, *range_left, *range_right);
                        let lo = self.normalize_bit_index(*r, *range_left, *range_right);
                        let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
                        Ok(self.module.slice(base, hi, lo))
                    }
                    ("indexed_up", Some(base_idx), Some(w)) => {
                        let lo = self.normalize_bit_index(*base_idx, *range_left, *range_right);
                        let hi = lo + (*w as u32) - 1;
                        Ok(self.module.slice(base, hi, lo))
                    }
                    ("indexed_down", Some(base_idx), Some(w)) => {
                        let hi = self.normalize_bit_index(*base_idx, *range_left, *range_right);
                        let lo = hi + 1 - (*w as u32);
                        Ok(self.module.slice(base, hi, lo))
                    }
                    ("indexed_up", None, _) => {
                        let w = self.module.width(base);
                        let shift = self.lower_expr(left.as_ref().unwrap(), env)?;
                        let shift = self.module.resize(shift, w, false);
                        let shifted = self.module.intern(w, ExprKind::Lshr(base, shift));
                        Ok(self.module.slice(shifted, *width - 1, 0))
                    }
                    _ => bail!("unsupported dynamic range select"),
                }
            }
            Expr::Call {
                name,
                width,
                signed,
                args,
            } => self.lower_system_call(name, *width, *signed, args, env),
            Expr::Unsupported { .. } => bail!("unsupported SystemVerilog expression"),
        }
    }

    fn lower_system_call(
        &mut self,
        name: &str,
        width: u32,
        signed: bool,
        args: &[Expr],
        env: &HashMap<SignalId, ExprId>,
    ) -> Result<ExprId> {
        match name {
            "$initstate" => Ok(self.get_or_create_initstate()),
            "$anyseq" | "$allseq" => {
                let sname = format!("$anyseq_{}", self.any_counter);
                self.any_counter += 1;
                let sig = self
                    .module
                    .add_signal(sname, width, signed, SignalKind::AnySeq);
                Ok(self.module.sig_expr(sig))
            }
            "$anyconst" | "$allconst" => {
                let sname = format!("$anyconst_{}", self.any_counter);
                self.any_counter += 1;
                let sig = self
                    .module
                    .add_signal(sname, width, signed, SignalKind::AnyConst);
                Ok(self.module.sig_expr(sig))
            }
            "$past" => {
                if args.is_empty() {
                    bail!("$past requires at least 1 argument");
                }
                let ticks = if args.len() >= 2 {
                    self.try_eval_const(&args[1], 32, false)
                        .and_then(|c| c.to_u64())
                        .unwrap_or(1) as usize
                } else {
                    1
                };
                let mut val = self.lower_expr(&args[0], env)?;
                for _ in 0..ticks {
                    val = self.make_past_reg(val, None);
                }
                Ok(self.module.resize(val, width, signed))
            }
            "$rose" => {
                let cur = self.lower_expr(&args[0], env)?;
                let cur_bit = self.module.slice(cur, 0, 0);
                let prev = self.make_past_reg(cur_bit, None);
                let not_prev = self.module.not(prev);
                Ok(self.module.intern(1, ExprKind::And(not_prev, cur_bit)))
            }
            "$fell" => {
                let cur = self.lower_expr(&args[0], env)?;
                let cur_bit = self.module.slice(cur, 0, 0);
                let prev = self.make_past_reg(cur_bit, None);
                let not_cur = self.module.not(cur_bit);
                Ok(self.module.intern(1, ExprKind::And(prev, not_cur)))
            }
            "$stable" => {
                let cur = self.lower_expr(&args[0], env)?;
                let prev = self.make_past_reg(cur, None);
                Ok(self.module.eq(cur, prev, false))
            }
            "$changed" => {
                let cur = self.lower_expr(&args[0], env)?;
                let prev = self.make_past_reg(cur, None);
                let eq = self.module.eq(cur, prev, false);
                Ok(self.module.not(eq))
            }
            "$signed" => {
                let v = self.lower_expr(&args[0], env)?;
                Ok(self.module.resize(v, width, true))
            }
            "$unsigned" => {
                let v = self.lower_expr(&args[0], env)?;
                Ok(self.module.resize(v, width, false))
            }
            "$onehot" => {
                let v = self.lower_expr(&args[0], env)?;
                let w = self.module.width(v);
                // x != 0 && (x & (x - 1)) == 0
                let zero = self.module.c_u64(w, 0);
                let one = self.module.c_u64(w, 1);
                let nonzero = self.module.intern(1, ExprKind::Ne(v, zero));
                let minus1 = self.module.intern(w, ExprKind::Sub(v, one));
                let masked = self.module.intern(w, ExprKind::And(v, minus1));
                let single = self.module.intern(1, ExprKind::Eq(masked, zero));
                Ok(self.module.intern(1, ExprKind::And(nonzero, single)))
            }
            "$onehot0" => {
                let v = self.lower_expr(&args[0], env)?;
                let w = self.module.width(v);
                let zero = self.module.c_u64(w, 0);
                let one = self.module.c_u64(w, 1);
                let minus1 = self.module.intern(w, ExprKind::Sub(v, one));
                let masked = self.module.intern(w, ExprKind::And(v, minus1));
                Ok(self.module.intern(1, ExprKind::Eq(masked, zero)))
            }
            other => bail!("unsupported system function call '{other}'"),
        }
    }

    fn try_eval_const(&self, expr: &Expr, target_width: u32, signed: bool) -> Option<ConstBits> {
        match expr {
            Expr::Const { width, bits, .. } => {
                let c = ConstBits::new(*width, bits.clone());
                Some(if signed {
                    c.resize_sext(target_width)
                } else {
                    c.resize_zext(target_width)
                })
            }
            Expr::Conv { operand, .. } => self.try_eval_const(operand, target_width, operand.is_signed()),
            Expr::Unary { op, operand, .. } if op == "Minus" => {
                let c = self.try_eval_const(operand, target_width, signed)?;
                let v = c.to_u128()?;
                let mask = if target_width == 128 {
                    u128::MAX
                } else {
                    (1u128 << target_width) - 1
                };
                Some(ConstBits::from_u128(target_width, v.wrapping_neg() & mask))
            }
            Expr::Binary { op, left, right, .. } => {
                let l = self.try_eval_const(left, target_width, signed)?.to_u128()?;
                let r = self.try_eval_const(right, target_width, signed)?.to_u128()?;
                let mask = if target_width == 128 {
                    u128::MAX
                } else {
                    (1u128 << target_width) - 1
                };
                let val = match op.as_str() {
                    "Add" => l.wrapping_add(r),
                    "Subtract" => l.wrapping_sub(r),
                    "Multiply" => l.wrapping_mul(r),
                    _ => return None,
                };
                Some(ConstBits::from_u128(target_width, val & mask))
            }
            _ => None,
        }
    }
}

pub fn lower_to_ir(design: &ElaboratedDesign) -> Result<Module> {
    Lowerer::new(&design.module).lower_design(design)
}
