use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use vf_ir::{ConstBits, ExprId, ExprKind as IrExpr, Ff, Module, Property, PropertyKind, SignalId, SignalKind};

use crate::ast::*;
use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractRole {
    /// Verify the module implements its contract: assume `require`, assert `invariant` & `ensure`.
    Provider,
    /// Verify the enclosing environment obeys the module's preconditions: assert `require`, assume `invariant` & `ensure`.
    Caller,
}

#[derive(Debug, Clone)]
pub struct LoweredContract {
    pub module: Module,
    pub xfree_targets: Vec<String>,
    pub secure_checks: Vec<LoweredSecureCheck>,
}

#[derive(Debug, Clone)]
pub struct LoweredSecureCheck {
    pub sources: Vec<String>,
    pub targets: Vec<String>,
    pub declassify: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct TypedVal {
    expr: ExprId,
    width: u32,
    signed: bool,
    /// If `Some(v)`, this expression came from an unsized integer literal and can adopt a partner's width if `v` fits.
    unsized_val: Option<u64>,
}

/// Strictly typecheck and lower a `.vfc` v1.0 `Contract` directly into a `vf_ir::Module`.
pub fn lower_contract_to_ir(
    base: &Module,
    contract: &Contract,
    role: ContractRole,
    param_values: &HashMap<String, i64>,
) -> Result<LoweredContract> {
    let mut ctx = LowerCtx::new(base.clone(), contract, role, param_values)?;
    ctx.lower_all()?;
    let mut m = ctx.module;
    m.topo_sort_comb().map_err(|e| anyhow!(e))?;
    Ok(LoweredContract {
        module: m,
        xfree_targets: ctx.xfree_targets,
        secure_checks: ctx.secure_checks,
    })
}

struct LowerCtx<'a> {
    module: Module,
    contract: &'a Contract,
    role: ContractRole,
    params: HashMap<String, i64>,
    ghost_sigs: HashMap<String, SignalId>,
    rst_expr: ExprId,
    active_expr: ExprId,
    /// Cached post-reset history valid flags: `hist_valid[k]` is true iff >= k active cycles elapsed before now.
    hist_valid: Vec<ExprId>,
    /// Counter for auxiliary registers.
    aux_counter: usize,
    /// Current `at_trigger` capture substitutions (non-empty only when lowering an `ensure` response).
    trigger_captures: Option<HashMap<usize, TypedVal>>,
    /// Counter tracking `at_trigger(...)` call index during a single `ensure` lowering pass.
    trigger_idx: usize,
    /// Max `prev` depth encountered while lowering the current expression.
    cur_max_prev: u32,
    xfree_targets: Vec<String>,
    secure_checks: Vec<LoweredSecureCheck>,
}

impl<'a> LowerCtx<'a> {
    fn new(
        mut module: Module,
        contract: &'a Contract,
        role: ContractRole,
        param_values: &HashMap<String, i64>,
    ) -> Result<Self> {
        // Validate clock signal exists if specified.
        if let Some(clk) = &contract.clock {
            if module.find_signal(&clk.signal).is_none() {
                bail!(
                    "clock signal '{}' not found in module '{}' at {}",
                    clk.signal,
                    module.name,
                    clk.span
                );
            }
        }

        // Validate parameters and `where` constraints.
        let mut params = param_values.clone();
        for p in &contract.params {
            let Some(&val) = params.get(&p.name) else {
                bail!(
                    "missing required contract parameter '{}' (pass `-G {}=<value>`) at {}",
                    p.name,
                    p.name,
                    p.span
                );
            };
            if let Some(cond) = &p.constraint {
                let ok = eval_const_i64(cond, &params)?;
                if ok == 0 {
                    bail!(
                        "parameter constraint failed for '{}' = {} at {}",
                        p.name,
                        val,
                        p.span
                    );
                }
            }
            params.insert(p.name.clone(), val);
        }

        // Register symbolic declarations (`$anyconst` / `$anyseq`).
        for s in &contract.symbolics {
            if module.find_signal(&s.name).is_some() {
                bail!(
                    "symbolic '{}' collides with existing signal at {}",
                    s.name,
                    s.span
                );
            }
            let kind = match s.kind {
                SymbolicKind::AnyConst => SignalKind::AnyConst,
                SymbolicKind::AnySeq => SignalKind::AnySeq,
            };
            module.add_signal(&s.name, s.width, s.signed, kind);
        }

        // Register ghost state registers.
        let mut ghost_sigs = HashMap::new();
        for g in &contract.ghosts {
            if module.find_signal(&g.name).is_some() {
                bail!(
                    "ghost '{}' collides with existing signal at {}",
                    g.name,
                    g.span
                );
            }
            let sid = module.add_signal(&g.name, g.width, g.signed, SignalKind::Reg);
            if let Some(init_e) = &g.init {
                let v = eval_const_i64(init_e, &params)?;
                let c = if g.signed {
                    ConstBits::from_i64(g.width, v)
                } else {
                    if v < 0 {
                        bail!("negative initializer {v} for unsigned ghost '{}' at {}", g.name, g.span);
                    }
                    ConstBits::from_u64(g.width, v as u64)
                };
                module.signal_mut(sid).init = Some(c);
            }
            ghost_sigs.insert(g.name.clone(), sid);
        }

        let zero_b = module.c_bool(false);
        let one_b = module.c_bool(true);
        let mut ctx = Self {
            module,
            contract,
            role,
            params,
            ghost_sigs,
            rst_expr: zero_b,
            active_expr: one_b,
            hist_valid: Vec::new(),
            aux_counter: 0,
            trigger_captures: None,
            trigger_idx: 0,
            cur_max_prev: 0,
            xfree_targets: Vec::new(),
            secure_checks: Vec::new(),
        };

        // Lower reset condition (`reset when <expr>`).
        if let Some(rst) = &contract.reset {
            let rv = ctx.lower_bool(&rst.cond)?;
            ctx.rst_expr = rv;
            ctx.active_expr = ctx.module.logic_not(rv);
        }
        ctx.hist_valid.push(ctx.active_expr);

        Ok(ctx)
    }

    fn lower_all(&mut self) -> Result<()> {
        // 1. Lower `update { ... }` blocks into FFs for declared ghost variables.
        self.lower_ghost_updates()?;

        // 2. Lower `require` clauses.
        let req_kind = match self.role {
            ContractRole::Provider => PropertyKind::Assume,
            ContractRole::Caller => PropertyKind::Assert,
        };
        for r in &self.contract.requires {
            self.cur_max_prev = 0;
            let cond = self.lower_bool(&r.expr)?;
            let hist_ok = self.get_hist_valid(self.cur_max_prev);
            let en = self.module.logic_and(self.active_expr, hist_ok);
            self.module.properties.push(Property {
                name: format!("require_{}", r.label),
                kind: req_kind,
                cond,
                en,
                source: Some(format!("{}", r.span)),
            });
        }

        // 3. Lower `invariant` clauses.
        let guar_kind = match self.role {
            ContractRole::Provider => PropertyKind::Assert,
            ContractRole::Caller => PropertyKind::Assume,
        };
        for inv in &self.contract.invariants {
            self.cur_max_prev = 0;
            let cond = self.lower_bool(&inv.expr)?;
            let hist_ok = self.get_hist_valid(self.cur_max_prev);
            let en = self.module.logic_and(self.active_expr, hist_ok);
            self.module.properties.push(Property {
                name: format!("invariant_{}", inv.label),
                kind: guar_kind,
                cond,
                en,
                source: Some(format!("{}", inv.span)),
            });
        }

        // 4. Lower `operation ... when ... { ensure ... }` clauses.
        for op in &self.contract.operations {
            self.lower_operation(op, guar_kind)?;
        }

        // 5. Lower `fair` & `eventually` liveness clauses.
        for f in &self.contract.fairs {
            self.cur_max_prev = 0;
            let cond = self.lower_bool(&f.expr)?;
            let one = self.module.c_bool(true);
            self.module.properties.push(Property {
                name: format!("fair_{}", f.label),
                kind: PropertyKind::Fair,
                cond,
                en: one,
                source: Some(format!("{}", f.span)),
            });
        }
        for ev in &self.contract.eventuallys {
            self.cur_max_prev = 0;
            let cond = self.lower_bool(&ev.expr)?;
            let one = self.module.c_bool(true);
            self.module.properties.push(Property {
                name: format!("eventually_{}", ev.label),
                kind: PropertyKind::Live,
                cond,
                en: one,
                source: Some(format!("{}", ev.span)),
            });
        }

        // 6. Lower `cover` clauses.
        for cov in &self.contract.covers {
            self.cur_max_prev = 0;
            let cond = self.lower_bool(&cov.expr)?;
            let hist_ok = self.get_hist_valid(self.cur_max_prev);
            let en = self.module.logic_and(self.active_expr, hist_ok);
            self.module.properties.push(Property {
                name: format!("cover_{}", cov.label),
                kind: PropertyKind::Cover,
                cond,
                en,
                source: Some(format!("{}", cov.span)),
            });
        }

        // 7. Lower built-in `use` protocol clauses.
        for u in &self.contract.uses {
            self.lower_use(u, guar_kind)?;
        }

        // 8. Collect `xfree` and `secure` clauses (validating signal existence!).
        for xf in &self.contract.xfrees {
            for t in &xf.targets {
                if self.module.find_signal(t).is_none() {
                    bail!("xfree target signal '{t}' not found at {}", xf.span);
                }
                self.xfree_targets.push(t.clone());
            }
        }
        for sec in &self.contract.secures {
            for s in &sec.sources {
                if self.module.find_signal(s).is_none() {
                    bail!("secure source signal '{s}' not found at {}", sec.span);
                }
            }
            for t in &sec.targets {
                if self.module.find_signal(t).is_none() {
                    bail!("secure target signal '{t}' not found at {}", sec.span);
                }
            }
            let declassify = sec.unless.as_ref().map(|e| crate::fmt::fmt_expr(e, 0));
            self.secure_checks.push(LoweredSecureCheck {
                sources: sec.sources.clone(),
                targets: sec.targets.clone(),
                declassify,
            });
        }

        Ok(())
    }

    fn lower_ghost_updates(&mut self) -> Result<()> {
        if self.contract.ghosts.is_empty() {
            if !self.contract.updates.is_empty() {
                bail!(
                    "`update` block declared without any `ghost` variables at {}",
                    self.contract.updates[0].span
                );
            }
            return Ok(());
        }

        // Default next value for each ghost is hold current value.
        let mut next_map: HashMap<SignalId, ExprId> = HashMap::new();
        for &sid in self.ghost_sigs.values() {
            next_map.insert(sid, self.module.sig_expr(sid));
        }

        for blk in &self.contract.updates {
            self.lower_update_stmts(&blk.stmts, &mut next_map)?;
        }

        for g in &self.contract.ghosts {
            let sid = self.ghost_sigs[&g.name];
            let init = self.module.signal(sid).init.clone();
            let body_next = next_map[&sid];
            // On reset, reload ghost init value if specified!
            let eff_next = if let Some(init_c) = &init {
                let ic = self.module.c_bits(init_c.clone());
                self.module.mux(self.rst_expr, ic, body_next)
            } else {
                body_next
            };
            self.module.ffs.push(Ff {
                signal: sid,
                next: eff_next,
                init,
                clock: None,
                async_reset: None,
            });
        }
        Ok(())
    }

    fn lower_update_stmts(
        &mut self,
        stmts: &[UpdateStmt],
        next_map: &mut HashMap<SignalId, ExprId>,
    ) -> Result<()> {
        for s in stmts {
            match s {
                UpdateStmt::Assign {
                    target,
                    value,
                    span,
                } => {
                    let Some(&sid) = self.ghost_sigs.get(target) else {
                        bail!("assignment target '{target}' at {span} is not a declared `ghost` variable");
                    };
                    let tw = self.module.signal(sid).width;
                    let ts = self.module.signal(sid).signed;
                    let rhs = self.lower_expr_expected(value, Some((tw, ts)))?;
                    if rhs.width != tw {
                        bail!(
                            "ghost assignment '{target} <= ...' width mismatch: expected {tw}-bit, got {}-bit at {span}",
                            rhs.width
                        );
                    }
                    next_map.insert(sid, rhs.expr);
                }
                UpdateStmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    let c = self.lower_bool(cond)?;
                    let mut t_map = next_map.clone();
                    let mut e_map = next_map.clone();
                    self.lower_update_stmts(then_body, &mut t_map)?;
                    self.lower_update_stmts(else_body, &mut e_map)?;
                    for (&sid, &tv) in &t_map {
                        let ev = e_map[&sid];
                        if tv != ev {
                            let merged = self.module.mux(c, tv, ev);
                            next_map.insert(sid, merged);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn lower_operation(&mut self, op: &Operation, guar_kind: PropertyKind) -> Result<()> {
        self.cur_max_prev = 0;
        let trig_raw = self.lower_bool(&op.trigger)?;
        let hist_ok = self.get_hist_valid(self.cur_max_prev);
        let trig_en = self.module.logic_and(self.active_expr, hist_ok);
        let eff_trig = self.module.logic_and(trig_raw, trig_en);

        for ens in &op.ensures {
            // Collect all `at_trigger(...)` expressions in `ens.response` in deterministic traversal order.
            let mut at_trig_exprs = Vec::new();
            collect_at_triggers(&ens.response, &mut at_trig_exprs);
            let mut captured_at_t0 = Vec::with_capacity(at_trig_exprs.len());
            for ae in &at_trig_exprs {
                captured_at_t0.push(self.lower_expr(ae)?);
            }

            let prop_name = format!("op_{}_{}", op.name, ens.label);
            match &ens.timing {
                EnsureTiming::After(n) => {
                    let n = *n as usize;
                    // Pipeline shift chain of length `n` (reset cancels in-flight obligations!).
                    let mut cur_trig = eff_trig;
                    let mut cur_caps: Vec<TypedVal> = captured_at_t0;
                    for _ in 0..n {
                        cur_trig = self.make_resettable_reg_1(cur_trig);
                        for cap in &mut cur_caps {
                            cap.expr = self.make_plain_reg(cap.expr);
                        }
                    }

                    // Bind `at_trigger` captures and lower response.
                    let cap_map: HashMap<usize, TypedVal> =
                        cur_caps.into_iter().enumerate().collect();
                    self.trigger_captures = Some(cap_map);
                    self.trigger_idx = 0;
                    let resp = self.lower_bool(&ens.response)?;
                    self.trigger_captures = None;

                    let en = self.module.logic_and(cur_trig, self.active_expr);
                    self.module.properties.push(Property {
                        name: prop_name,
                        kind: guar_kind,
                        cond: resp,
                        en,
                        source: Some(format!("{}", ens.span)),
                    });
                }
                EnsureTiming::Within(a, b) => {
                    let a = *a as usize;
                    let b = *b as usize;
                    // Track overlapping independent obligations across slots `1..=b`.
                    // Keep `prev_trig` separate from `prev_met` so early completion (before deadline `b`)
                    // never makes the deadline assertion's enable guard falsely vacuous!
                    let mut prev_trig = eff_trig;
                    let mut prev_met = self.module.c_bool(false);
                    let mut prev_caps = captured_at_t0;

                    for slot in 1..=b {
                        let slot_trig = self.make_resettable_reg_1(prev_trig);
                        let slot_met = self.make_resettable_reg_1(prev_met);
                        let mut slot_caps = Vec::with_capacity(prev_caps.len());
                        for cap in &prev_caps {
                            slot_caps.push(TypedVal {
                                expr: self.make_plain_reg(cap.expr),
                                ..*cap
                            });
                        }

                        if slot >= a {
                            let cap_map: HashMap<usize, TypedVal> =
                                slot_caps.iter().copied().enumerate().collect();
                            self.trigger_captures = Some(cap_map);
                            self.trigger_idx = 0;
                            let resp = self.lower_bool(&ens.response)?;
                            self.trigger_captures = None;
                            let met_now = self.module.logic_or(slot_met, resp);

                            if slot == b {
                                // Deadline reached: response MUST have held at some cycle in `a..=b`!
                                let en = self.module.logic_and(slot_trig, self.active_expr);
                                self.module.properties.push(Property {
                                    name: prop_name.clone(),
                                    kind: guar_kind,
                                    cond: met_now,
                                    en,
                                    source: Some(format!("{}", ens.span)),
                                });
                            } else {
                                prev_trig = slot_trig;
                                prev_met = met_now;
                                prev_caps = slot_caps;
                            }
                        } else {
                            prev_trig = slot_trig;
                            prev_met = slot_met;
                            prev_caps = slot_caps;
                        }
                    }
                }
                EnsureTiming::Until(ev_expr) => {
                    let ev = self.lower_bool(ev_expr)?;
                    let not_ev = self.module.logic_not(ev);

                    // Create active-until tracking FF + capture FFs (set on eff_trig, cleared on reset or ev).
                    let act_sig = self.add_aux_reg("until_act", 1, Some(ConstBits::zero(1)));
                    let act_e = self.module.sig_expr(act_sig);

                    let mut cap_regs = Vec::with_capacity(captured_at_t0.len());
                    for (i, cap) in captured_at_t0.iter().enumerate() {
                        let csig = self.add_aux_reg(&format!("until_cap{i}"), cap.width, None);
                        let ce = self.module.sig_expr(csig);
                        let next_c = self.module.mux(eff_trig, cap.expr, ce);
                        self.module.ffs.push(Ff {
                            signal: csig,
                            next: next_c,
                            init: None,
                            clock: None,
                            async_reset: None,
                        });
                        cap_regs.push(TypedVal { expr: ce, ..*cap });
                    }

                    // Next active state: reset -> 0, eff_trig -> 1, ev -> 0, else hold.
                    let zero = self.module.c_bool(false);
                    let one = self.module.c_bool(true);
                    let after_ev = self.module.mux(ev, zero, act_e);
                    let after_trig = self.module.mux(eff_trig, one, after_ev);
                    let next_act = self.module.mux(self.rst_expr, zero, after_trig);
                    self.module.ffs.push(Ff {
                        signal: act_sig,
                        next: next_act,
                        init: Some(ConstBits::zero(1)),
                        clock: None,
                        async_reset: None,
                    });

                    let cap_map: HashMap<usize, TypedVal> =
                        cap_regs.into_iter().enumerate().collect();
                    self.trigger_captures = Some(cap_map);
                    self.trigger_idx = 0;
                    let resp = self.lower_bool(&ens.response)?;
                    self.trigger_captures = None;

                    let en1 = self.module.logic_and(act_e, self.active_expr);
                    let en = self.module.logic_and(en1, not_ev);
                    self.module.properties.push(Property {
                        name: prop_name,
                        kind: guar_kind,
                        cond: resp,
                        en,
                        source: Some(format!("{}", ens.span)),
                    });
                }
                EnsureTiming::Eventually => {
                    if !at_trig_exprs.is_empty() {
                        bail!(
                            "`at_trigger(...)` is not supported in unbounded `eventually` ensure at {}",
                            ens.span
                        );
                    }
                    let resp = self.lower_bool(&ens.response)?;
                    let pend_sig = self.add_aux_reg("ev_pend", 1, Some(ConstBits::zero(1)));
                    let pend_e = self.module.sig_expr(pend_sig);
                    let zero = self.module.c_bool(false);
                    let one = self.module.c_bool(true);
                    let cleared = self.module.mux(resp, zero, pend_e);
                    let set = self.module.mux(eff_trig, one, cleared);
                    let next_pend = self.module.mux(self.rst_expr, zero, set);
                    self.module.ffs.push(Ff {
                        signal: pend_sig,
                        next: next_pend,
                        init: Some(ConstBits::zero(1)),
                        clock: None,
                        async_reset: None,
                    });
                    // Liveness: infinitely often `(!pend || resp)` (cannot stay pending forever!).
                    let not_pend = self.module.logic_not(pend_e);
                    let ok = self.module.logic_or(not_pend, resp);
                    self.module.properties.push(Property {
                        name: prop_name,
                        kind: PropertyKind::Live,
                        cond: ok,
                        en: one,
                        source: Some(format!("{}", ens.span)),
                    });
                }
            }
        }
        Ok(())
    }

    fn lower_use(&mut self, u: &UseClause, guar_kind: PropertyKind) -> Result<()> {
        match u.protocol.as_str() {
            "handshake" => {
                if u.args.len() != 3 {
                    bail!(
                        "`use handshake(valid, ready, data)` expects 3 arguments at {}",
                        u.span
                    );
                }
                let valid = self.lower_bool(&u.args[0])?;
                let ready = self.lower_bool(&u.args[1])?;
                let data = self.lower_expr(&u.args[2])?;
                self.emit_handshake_props("handshake", valid, ready, data.expr, guar_kind, u.span);
            }
            "apb" => {
                if u.args.len() != 7 {
                    bail!(
                        "`use apb(psel, penable, pwrite, paddr, pwdata, pready, prdata)` expects 7 arguments at {}",
                        u.span
                    );
                }
                let psel = self.lower_bool(&u.args[0])?;
                let penable = self.lower_bool(&u.args[1])?;
                let pwrite = self.lower_bool(&u.args[2])?;
                let paddr = self.lower_expr(&u.args[3])?.expr;
                let pwdata = self.lower_expr(&u.args[4])?.expr;
                let pready = self.lower_bool(&u.args[5])?;
                let _prdata = self.lower_expr(&u.args[6])?.expr;

                // 1. PENABLE implies PSEL
                let not_pen = self.module.logic_not(penable);
                let pen_imp_psel = self.module.logic_or(not_pen, psel);
                self.module.properties.push(Property {
                    name: "use_apb_penable_implies_psel".into(),
                    kind: guar_kind,
                    cond: pen_imp_psel,
                    en: self.active_expr,
                    source: Some(format!("{}", u.span)),
                });

                // Track previous cycle signals (guarded by hist_valid[1]).
                let hist1 = self.get_hist_valid(1);
                let prev_psel = self.make_resettable_reg_1(psel);
                let prev_pen = self.make_resettable_reg_1(penable);
                let prev_pwrite = self.make_plain_reg(pwrite);
                let prev_paddr = self.make_plain_reg(paddr);
                let prev_pwdata = self.make_plain_reg(pwdata);
                let prev_pready = self.make_resettable_reg_1(pready);

                // SETUP -> ACCESS or ACCESS wait-state: psel && penable && stable(paddr, pwrite, pwdata)
                let not_prev_pen = self.module.logic_not(prev_pen);
                let was_setup = self.module.logic_and(prev_psel, not_prev_pen);
                let not_prev_rdy = self.module.logic_not(prev_pready);
                let was_wait1 = self.module.logic_and(prev_psel, prev_pen);
                let was_wait = self.module.logic_and(was_wait1, not_prev_rdy);
                let hold_cond = self.module.logic_or(was_setup, was_wait);
                let en1 = self.module.logic_and(self.active_expr, hist1);
                let en = self.module.logic_and(en1, hold_cond);

                let in_access = self.module.logic_and(psel, penable);
                let addr_eq = self.module.eq(paddr, prev_paddr, false);
                let wr_eq = self.module.eq(pwrite, prev_pwrite, false);
                let wd_eq = self.module.eq(pwdata, prev_pwdata, false);
                let not_prev_wr = self.module.logic_not(prev_pwrite);
                let wd_ok = self.module.logic_or(not_prev_wr, wd_eq);

                let c1 = self.module.logic_and(in_access, addr_eq);
                let c2 = self.module.logic_and(c1, wr_eq);
                let stable_all = self.module.logic_and(c2, wd_ok);

                self.module.properties.push(Property {
                    name: "use_apb_phase_stability".into(),
                    kind: guar_kind,
                    cond: stable_all,
                    en,
                    source: Some(format!("{}", u.span)),
                });

                let xfer = self.module.logic_and(in_access, pready);
                self.module.properties.push(Property {
                    name: "use_apb_cover_transfer".into(),
                    kind: PropertyKind::Cover,
                    cond: xfer,
                    en: self.active_expr,
                    source: Some(format!("{}", u.span)),
                });
            }
            "axi4_lite" => {
                if u.args.len() != 17 {
                    bail!(
                        "`use axi4_lite(awv, awr, awaddr, wv, wr, wdata, wstrb, bv, br, bresp, arv, arr, araddr, rv, rr, rdata, rresp)` expects 17 arguments at {}",
                        u.span
                    );
                }
                let awv = self.lower_bool(&u.args[0])?;
                let awr = self.lower_bool(&u.args[1])?;
                let awaddr = self.lower_expr(&u.args[2])?.expr;
                self.emit_handshake_props("axi_aw", awv, awr, awaddr, guar_kind, u.span);

                let wv = self.lower_bool(&u.args[3])?;
                let wr = self.lower_bool(&u.args[4])?;
                let wdata = self.lower_expr(&u.args[5])?.expr;
                let wstrb = self.lower_expr(&u.args[6])?.expr;
                let wpay = self.module.concat(vec![wdata, wstrb]);
                self.emit_handshake_props("axi_w", wv, wr, wpay, guar_kind, u.span);

                let bv = self.lower_bool(&u.args[7])?;
                let br = self.lower_bool(&u.args[8])?;
                let bresp = self.lower_expr(&u.args[9])?.expr;
                self.emit_handshake_props("axi_b", bv, br, bresp, guar_kind, u.span);

                let arv = self.lower_bool(&u.args[10])?;
                let arr = self.lower_bool(&u.args[11])?;
                let araddr = self.lower_expr(&u.args[12])?.expr;
                self.emit_handshake_props("axi_ar", arv, arr, araddr, guar_kind, u.span);

                let rv = self.lower_bool(&u.args[13])?;
                let rr = self.lower_bool(&u.args[14])?;
                let rdata = self.lower_expr(&u.args[15])?.expr;
                let rresp = self.lower_expr(&u.args[16])?.expr;
                let rpay = self.module.concat(vec![rdata, rresp]);
                self.emit_handshake_props("axi_r", rv, rr, rpay, guar_kind, u.span);
            }
            "fifo_scoreboard" => {
                if u.args.len() != 7 {
                    bail!(
                        "`use fifo_scoreboard(wr_en, wr_data, rd_en, rd_data, full, empty, DEPTH)` expects 7 arguments at {}",
                        u.span
                    );
                }
                let wr_en = self.lower_bool(&u.args[0])?;
                let wr_data = self.lower_expr(&u.args[1])?;
                let rd_en = self.lower_bool(&u.args[2])?;
                let rd_data = self.lower_expr(&u.args[3])?;
                let full = self.lower_bool(&u.args[4])?;
                let empty = self.lower_bool(&u.args[5])?;
                let depth = eval_const_i64(&u.args[6], &self.params)?;
                if depth <= 0 {
                    bail!("fifo_scoreboard DEPTH must be > 0 at {}", u.span);
                }
                let depth_u = depth as u64;
                let cnt_w = (64 - depth_u.leading_zeros()).max(2);

                let not_full = self.module.logic_not(full);
                let not_empty = self.module.logic_not(empty);
                let do_wr = self.module.logic_and(wr_en, not_full);
                let do_rd = self.module.logic_and(rd_en, not_empty);

                let cnt_sig = self.add_aux_reg("fifo_cnt", cnt_w, Some(ConstBits::zero(cnt_w)));
                let cnt_e = self.module.sig_expr(cnt_sig);
                let one_c = self.module.c_u64(cnt_w, 1);
                let zero_c = self.module.c_u64(cnt_w, 0);
                let cnt_inc = self.module.intern(cnt_w, IrExpr::Add(cnt_e, one_c));
                let cnt_dec = self.module.intern(cnt_w, IrExpr::Sub(cnt_e, one_c));
                let only_wr = {
                    let nr = self.module.logic_not(do_rd);
                    self.module.logic_and(do_wr, nr)
                };
                let only_rd = {
                    let nw = self.module.logic_not(do_wr);
                    self.module.logic_and(do_rd, nw)
                };
                let c_step1 = self.module.mux(only_rd, cnt_dec, cnt_e);
                let c_step2 = self.module.mux(only_wr, cnt_inc, c_step1);
                let c_next = self.module.mux(self.rst_expr, zero_c, c_step2);
                self.module.ffs.push(Ff {
                    signal: cnt_sig,
                    next: c_next,
                    init: Some(ConstBits::zero(cnt_w)),
                    clock: None,
                    async_reset: None,
                });

                // Symbolic tracked value (`$anyconst`)
                let magic_name = format!("$dsl_fifo_magic_{}", self.aux_counter);
                self.aux_counter += 1;
                let magic_sig =
                    self.module
                        .add_signal(magic_name, wr_data.width, false, SignalKind::AnyConst);
                let magic_e = self.module.sig_expr(magic_sig);

                let act_sig = self.add_aux_reg("fifo_trk", 1, Some(ConstBits::zero(1)));
                let pos_sig = self.add_aux_reg("fifo_pos", cnt_w, Some(ConstBits::zero(cnt_w)));
                let act_e = self.module.sig_expr(act_sig);
                let pos_e = self.module.sig_expr(pos_sig);

                let not_act = self.module.logic_not(act_e);
                let wr_match = self.module.eq(wr_data.expr, magic_e, false);
                let cap_1 = self.module.logic_and(not_act, do_wr);
                let capture = self.module.logic_and(cap_1, wr_match);

                let pos_is_0 = self.module.eq(pos_e, zero_c, false);
                let not_pos_0 = self.module.logic_not(pos_is_0);
                let act_rd = self.module.logic_and(act_e, do_rd);
                let zero_b = self.module.c_bool(false);
                let one_b = self.module.c_bool(true);

                let act_after_rd = self.module.mux(act_rd, not_pos_0, act_e);
                let act_after_cap = self.module.mux(capture, one_b, act_after_rd);
                let act_next = self.module.mux(self.rst_expr, zero_b, act_after_cap);
                self.module.ffs.push(Ff {
                    signal: act_sig,
                    next: act_next,
                    init: Some(ConstBits::zero(1)),
                    clock: None,
                    async_reset: None,
                });

                let pos_dec = self.module.intern(cnt_w, IrExpr::Sub(pos_e, one_c));
                let cap_pos = self.module.mux(do_rd, cnt_dec, cnt_e);
                let pos_after_rd = self.module.mux(act_rd, pos_dec, pos_e);
                let pos_after_cap = self.module.mux(capture, cap_pos, pos_after_rd);
                let pos_next = self.module.mux(self.rst_expr, zero_c, pos_after_cap);
                self.module.ffs.push(Ff {
                    signal: pos_sig,
                    next: pos_next,
                    init: Some(ConstBits::zero(cnt_w)),
                    clock: None,
                    async_reset: None,
                });

                // Assertions: empty == (cnt == 0), full == (cnt == DEPTH), data integrity!
                let depth_c = self.module.c_u64(cnt_w, depth_u);
                let cnt_is_0 = self.module.eq(cnt_e, zero_c, false);
                let cnt_is_depth = self.module.eq(cnt_e, depth_c, false);
                let empty_ok = self.module.eq(empty, cnt_is_0, false);
                let full_ok = self.module.eq(full, cnt_is_depth, false);

                self.module.properties.push(Property {
                    name: "use_fifo_empty_flag".into(),
                    kind: guar_kind,
                    cond: empty_ok,
                    en: self.active_expr,
                    source: Some(format!("{}", u.span)),
                });
                self.module.properties.push(Property {
                    name: "use_fifo_full_flag".into(),
                    kind: guar_kind,
                    cond: full_ok,
                    en: self.active_expr,
                    source: Some(format!("{}", u.span)),
                });

                let head_pop = self.module.logic_and(act_rd, pos_is_0);
                let head_en = self.module.logic_and(self.active_expr, head_pop);
                let data_ok = self.module.eq(rd_data.expr, magic_e, false);
                self.module.properties.push(Property {
                    name: "use_fifo_data_integrity".into(),
                    kind: guar_kind,
                    cond: data_ok,
                    en: head_en,
                    source: Some(format!("{}", u.span)),
                });
            }
            other => bail!(
                "unknown protocol '{other}' in `use` clause at {} (supported: handshake, apb, axi4_lite, fifo_scoreboard)",
                u.span
            ),
        }
        Ok(())
    }

    fn emit_handshake_props(
        &mut self,
        prefix: &str,
        valid: ExprId,
        ready: ExprId,
        data: ExprId,
        kind: PropertyKind,
        span: Span,
    ) {
        let hist1 = self.get_hist_valid(1);
        let prev_v = self.make_resettable_reg_1(valid);
        let prev_r = self.make_resettable_reg_1(ready);
        let prev_d = self.make_plain_reg(data);

        let not_pr = self.module.logic_not(prev_r);
        let stalled = self.module.logic_and(prev_v, not_pr);
        let en1 = self.module.logic_and(self.active_expr, hist1);
        let en = self.module.logic_and(en1, stalled);

        let data_eq = self.module.eq(data, prev_d, false);
        let both_ok = self.module.logic_and(valid, data_eq);
        self.module.properties.push(Property {
            name: format!("use_{prefix}_stability"),
            kind,
            cond: both_ok,
            en,
            source: Some(format!("{span}")),
        });

        let xfer = self.module.logic_and(valid, ready);
        self.module.properties.push(Property {
            name: format!("use_{prefix}_cover_xfer"),
            kind: PropertyKind::Cover,
            cond: xfer,
            en: self.active_expr,
            source: Some(format!("{span}")),
        });
    }

    // ---- Expression typing & lowering ----

    fn lower_bool(&mut self, expr: &Expr) -> Result<ExprId> {
        let tv = self.lower_expr(expr)?;
        Ok(self.module.to_bool(tv.expr))
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<TypedVal> {
        self.lower_expr_expected(expr, None)
    }

    fn lower_expr_expected(
        &mut self,
        expr: &Expr,
        expected: Option<(u32, bool)>,
    ) -> Result<TypedVal> {
        match &expr.kind {
            ExprKind::Bool(b) => Ok(TypedVal {
                expr: self.module.c_bool(*b),
                width: 1,
                signed: false,
                unsized_val: Some(u64::from(*b)),
            }),
            ExprKind::Int(v) => {
                if let Some((w, s)) = expected {
                    self.check_unsized_fits(*v, w, s, expr.span)?;
                    Ok(TypedVal {
                        expr: self.module.c_u64(w, *v),
                        width: w,
                        signed: s,
                        unsized_val: Some(*v),
                    })
                } else {
                    let w = (64 - v.leading_zeros()).max(1);
                    Ok(TypedVal {
                        expr: self.module.c_u64(w, *v),
                        width: w,
                        signed: false,
                        unsized_val: Some(*v),
                    })
                }
            }
            ExprKind::SizedLit {
                width,
                signed,
                bits,
            } => Ok(TypedVal {
                expr: self.module.c_bits(ConstBits::new(*width, bits.clone())),
                width: *width,
                signed: *signed,
                unsized_val: None,
            }),
            ExprKind::Ident(name) => {
                if let Some(&pv) = self.params.get(name) {
                    if let Some((w, s)) = expected {
                        if !s && pv < 0 {
                            bail!("negative parameter '{name}' ({pv}) used in unsigned context at {}", expr.span);
                        }
                        if !s {
                            self.check_unsized_fits(pv as u64, w, false, expr.span)?;
                        }
                        let c = if s {
                            ConstBits::from_i64(w, pv)
                        } else {
                            ConstBits::from_u64(w, pv as u64)
                        };
                        return Ok(TypedVal {
                            expr: self.module.c_bits(c),
                            width: w,
                            signed: s,
                            unsized_val: (pv >= 0).then_some(pv as u64),
                        });
                    }
                    let u = pv.max(0) as u64;
                    let w = (64 - u.leading_zeros()).max(1);
                    return Ok(TypedVal {
                        expr: self.module.c_u64(w, u),
                        width: w,
                        signed: pv < 0,
                        unsized_val: (pv >= 0).then_some(u),
                    });
                }
                let Some(sid) = self.module.find_signal(name) else {
                    bail!(
                        "unknown signal or parameter '{name}' in module '{}' at {}",
                        self.module.name,
                        expr.span
                    );
                };
                let s = self.module.signal(sid);
                let (w, signed) = (s.width, s.signed);
                Ok(TypedVal {
                    expr: self.module.sig_expr(sid),
                    width: w,
                    signed,
                    unsized_val: None,
                })
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnOp::LogNot => {
                    let v = self.lower_bool(inner)?;
                    Ok(TypedVal {
                        expr: self.module.logic_not(v),
                        width: 1,
                        signed: false,
                        unsized_val: None,
                    })
                }
                UnOp::BitNot => {
                    let v = self.lower_expr_expected(inner, expected)?;
                    Ok(TypedVal {
                        expr: self.module.not(v.expr),
                        width: v.width,
                        signed: v.signed,
                        unsized_val: None,
                    })
                }
                UnOp::Neg => {
                    let v = self.lower_expr_expected(inner, expected)?;
                    Ok(TypedVal {
                        expr: self.module.intern(v.width, IrExpr::Neg(v.expr)),
                        width: v.width,
                        signed: v.signed,
                        unsized_val: None,
                    })
                }
            },
            ExprKind::Binary { op, left, right } => match op {
                BinOp::Implies => {
                    let a = self.lower_bool(left)?;
                    let b = self.lower_bool(right)?;
                    let na = self.module.logic_not(a);
                    Ok(TypedVal {
                        expr: self.module.logic_or(na, b),
                        width: 1,
                        signed: false,
                        unsized_val: None,
                    })
                }
                BinOp::LogOr => {
                    let a = self.lower_bool(left)?;
                    let b = self.lower_bool(right)?;
                    Ok(TypedVal {
                        expr: self.module.logic_or(a, b),
                        width: 1,
                        signed: false,
                        unsized_val: None,
                    })
                }
                BinOp::LogAnd => {
                    let a = self.lower_bool(left)?;
                    let b = self.lower_bool(right)?;
                    Ok(TypedVal {
                        expr: self.module.logic_and(a, b),
                        width: 1,
                        signed: false,
                        unsized_val: None,
                    })
                }
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let (l, r, _w, signed) = self.lower_matched_pair(left, right, expr.span)?;
                    let res = match op {
                        BinOp::Eq => self.module.intern(1, IrExpr::Eq(l, r)),
                        BinOp::Ne => self.module.intern(1, IrExpr::Ne(l, r)),
                        BinOp::Lt => self.module.intern(
                            1,
                            if signed {
                                IrExpr::Slt(l, r)
                            } else {
                                IrExpr::Ult(l, r)
                            },
                        ),
                        BinOp::Le => self.module.intern(
                            1,
                            if signed {
                                IrExpr::Sle(l, r)
                            } else {
                                IrExpr::Ule(l, r)
                            },
                        ),
                        BinOp::Gt => self.module.intern(
                            1,
                            if signed {
                                IrExpr::Slt(r, l)
                            } else {
                                IrExpr::Ult(r, l)
                            },
                        ),
                        BinOp::Ge => self.module.intern(
                            1,
                            if signed {
                                IrExpr::Sle(r, l)
                            } else {
                                IrExpr::Ule(r, l)
                            },
                        ),
                        _ => unreachable!(),
                    };
                    Ok(TypedVal {
                        expr: res,
                        width: 1,
                        signed: false,
                        unsized_val: None,
                    })
                }
                BinOp::Shl | BinOp::Shr => {
                    let lv = self.lower_expr_expected(left, expected)?;
                    let rv = self.lower_expr_expected(right, Some((lv.width, false)))?;
                    let r_resized = self.module.resize(rv.expr, lv.width, false);
                    let res = match op {
                        BinOp::Shl => self.module.intern(lv.width, IrExpr::Shl(lv.expr, r_resized)),
                        BinOp::Shr => self.module.intern(
                            lv.width,
                            if lv.signed {
                                IrExpr::Ashr(lv.expr, r_resized)
                            } else {
                                IrExpr::Lshr(lv.expr, r_resized)
                            },
                        ),
                        _ => unreachable!(),
                    };
                    Ok(TypedVal {
                        expr: res,
                        width: lv.width,
                        signed: lv.signed,
                        unsized_val: None,
                    })
                }
                BinOp::BitOr
                | BinOp::BitXor
                | BinOp::BitAnd
                | BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod => {
                    let (l, r, w, signed) = self.lower_matched_pair(left, right, expr.span)?;
                    let kind = match op {
                        BinOp::BitOr => IrExpr::Or(l, r),
                        BinOp::BitXor => IrExpr::Xor(l, r),
                        BinOp::BitAnd => IrExpr::And(l, r),
                        BinOp::Add => IrExpr::Add(l, r),
                        BinOp::Sub => IrExpr::Sub(l, r),
                        BinOp::Mul => IrExpr::Mul(l, r),
                        BinOp::Div => {
                            if signed {
                                IrExpr::SDiv(l, r)
                            } else {
                                IrExpr::UDiv(l, r)
                            }
                        }
                        BinOp::Mod => {
                            if signed {
                                IrExpr::SMod(l, r)
                            } else {
                                IrExpr::UMod(l, r)
                            }
                        }
                        _ => unreachable!(),
                    };
                    Ok(TypedVal {
                        expr: self.module.intern(w, kind),
                        width: w,
                        signed,
                        unsized_val: None,
                    })
                }
            },
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                let c = self.lower_bool(cond)?;
                let (t, e, w, signed) = self.lower_matched_pair(then_expr, else_expr, expr.span)?;
                Ok(TypedVal {
                    expr: self.module.mux(c, t, e),
                    width: w,
                    signed,
                    unsized_val: None,
                })
            }
            ExprKind::Slice {
                expr: inner,
                hi,
                lo,
            } => {
                let v = self.lower_expr(inner)?;
                if *hi >= v.width {
                    bail!(
                        "slice [{hi}:{lo}] out of bounds for {}-bit expression at {}",
                        v.width,
                        expr.span
                    );
                }
                Ok(TypedVal {
                    expr: self.module.slice(v.expr, *hi, *lo),
                    width: hi - lo + 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            ExprKind::Concat(parts) => {
                let mut ir_parts = Vec::with_capacity(parts.len());
                let mut total_w = 0u32;
                for p in parts {
                    let tv = self.lower_expr(p)?;
                    total_w += tv.width;
                    ir_parts.push(tv.expr);
                }
                Ok(TypedVal {
                    expr: self.module.concat(ir_parts),
                    width: total_w,
                    signed: false,
                    unsized_val: None,
                })
            }
            ExprKind::Call { name, args } => self.lower_call(name, args, expr.span),
        }
    }

    fn lower_call(&mut self, name: &str, args: &[Expr], span: Span) -> Result<TypedVal> {
        match name {
            "at_trigger" => {
                if args.len() != 1 {
                    bail!("`at_trigger(expr)` expects 1 argument at {span}");
                }
                let Some(caps) = &self.trigger_captures else {
                    bail!("`at_trigger(...)` is only valid inside an `ensure` clause at {span}");
                };
                let idx = self.trigger_idx;
                self.trigger_idx += 1;
                caps.get(&idx).copied().ok_or_else(|| {
                    anyhow!("internal error: missing at_trigger capture #{idx} at {span}")
                })
            }
            "unsigned" => {
                if args.len() != 1 {
                    bail!("`unsigned(expr)` expects 1 argument at {span}");
                }
                let mut v = self.lower_expr(&args[0])?;
                v.signed = false;
                Ok(v)
            }
            "signed" => {
                if args.len() != 1 {
                    bail!("`signed(expr)` expects 1 argument at {span}");
                }
                let mut v = self.lower_expr(&args[0])?;
                v.signed = true;
                Ok(v)
            }
            "resize" => {
                if args.len() != 2 {
                    bail!("`resize(expr, width)` expects 2 arguments at {span}");
                }
                let w = eval_const_i64(&args[1], &self.params)?;
                if w <= 0 || w > 256 {
                    bail!("`resize` target width {w} must be in 1..=256 at {span}");
                }
                let w32 = w as u32;
                let v = self.lower_expr(&args[0])?;
                Ok(TypedVal {
                    expr: self.module.resize(v.expr, w32, v.signed),
                    width: w32,
                    signed: v.signed,
                    unsized_val: None,
                })
            }
            "ite" => {
                if args.len() != 3 {
                    bail!("`ite(cond, then, else)` expects 3 arguments at {span}");
                }
                let c = self.lower_bool(&args[0])?;
                let (t, e, w, signed) = self.lower_matched_pair(&args[1], &args[2], span)?;
                Ok(TypedVal {
                    expr: self.module.mux(c, t, e),
                    width: w,
                    signed,
                    unsized_val: None,
                })
            }
            "prev" => {
                if args.is_empty() || args.len() > 2 {
                    bail!("`prev(expr [, cycles])` expects 1 or 2 arguments at {span}");
                }
                let cycles = if args.len() == 2 {
                    let n = eval_const_i64(&args[1], &self.params)?;
                    if n <= 0 || n > 64 {
                        bail!("`prev` cycle count {n} must be in 1..=64 at {span}");
                    }
                    n as u32
                } else {
                    1
                };
                self.cur_max_prev = self.cur_max_prev.max(cycles);
                let mut v = self.lower_expr(&args[0])?;
                for _ in 0..cycles {
                    v.expr = self.make_plain_reg(v.expr);
                }
                v.unsized_val = None;
                Ok(v)
            }
            "rose" => {
                if args.len() != 1 {
                    bail!("`rose(expr)` expects 1 argument at {span}");
                }
                self.cur_max_prev = self.cur_max_prev.max(1);
                let cur = self.lower_bool(&args[0])?;
                let prev = self.make_plain_reg(cur);
                let not_prev = self.module.logic_not(prev);
                Ok(TypedVal {
                    expr: self.module.logic_and(cur, not_prev),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "fell" => {
                if args.len() != 1 {
                    bail!("`fell(expr)` expects 1 argument at {span}");
                }
                self.cur_max_prev = self.cur_max_prev.max(1);
                let cur = self.lower_bool(&args[0])?;
                let prev = self.make_plain_reg(cur);
                let not_cur = self.module.logic_not(cur);
                Ok(TypedVal {
                    expr: self.module.logic_and(prev, not_cur),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "stable" => {
                if args.len() != 1 {
                    bail!("`stable(expr)` expects 1 argument at {span}");
                }
                self.cur_max_prev = self.cur_max_prev.max(1);
                let cur = self.lower_expr(&args[0])?;
                let prev = self.make_plain_reg(cur.expr);
                Ok(TypedVal {
                    expr: self.module.eq(cur.expr, prev, false),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "changed" => {
                if args.len() != 1 {
                    bail!("`changed(expr)` expects 1 argument at {span}");
                }
                self.cur_max_prev = self.cur_max_prev.max(1);
                let cur = self.lower_expr(&args[0])?;
                let prev = self.make_plain_reg(cur.expr);
                let eq = self.module.eq(cur.expr, prev, false);
                Ok(TypedVal {
                    expr: self.module.logic_not(eq),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "onehot" => {
                if args.len() != 1 {
                    bail!("`onehot(expr)` expects 1 argument at {span}");
                }
                let v = self.lower_expr(&args[0])?;
                let nz = self.module.to_bool(v.expr);
                let one = self.module.c_u64(v.width, 1);
                let vm1 = self.module.intern(v.width, IrExpr::Sub(v.expr, one));
                let masked = self.module.intern(v.width, IrExpr::And(v.expr, vm1));
                let zero = self.module.c_u64(v.width, 0);
                let oh0 = self.module.eq(masked, zero, false);
                Ok(TypedVal {
                    expr: self.module.logic_and(nz, oh0),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "onehot0" => {
                if args.len() != 1 {
                    bail!("`onehot0(expr)` expects 1 argument at {span}");
                }
                let v = self.lower_expr(&args[0])?;
                let one = self.module.c_u64(v.width, 1);
                let vm1 = self.module.intern(v.width, IrExpr::Sub(v.expr, one));
                let masked = self.module.intern(v.width, IrExpr::And(v.expr, vm1));
                let zero = self.module.c_u64(v.width, 0);
                Ok(TypedVal {
                    expr: self.module.eq(masked, zero, false),
                    width: 1,
                    signed: false,
                    unsized_val: None,
                })
            }
            "countones" => {
                if args.len() != 1 {
                    bail!("`countones(expr)` expects 1 argument at {span}");
                }
                let v = self.lower_expr(&args[0])?;
                let out_w = (32 - v.width.leading_zeros()).max(1);
                let mut acc = self.module.c_u64(out_w, 0);
                for b in 0..v.width {
                    let bit = self.module.slice(v.expr, b, b);
                    let ext = self.module.resize(bit, out_w, false);
                    acc = self.module.intern(out_w, IrExpr::Add(acc, ext));
                }
                Ok(TypedVal {
                    expr: acc,
                    width: out_w,
                    signed: false,
                    unsized_val: None,
                })
            }
            other => bail!("unknown function '{other}' in contract expression at {span}"),
        }
    }

    /// Strictly match two operand types: reject mixed signed/unsigned and silent width mismatches,
    /// while allowing unsized integer literals/parameters to adapt when their value fits cleanly.
    fn lower_matched_pair(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<(ExprId, ExprId, u32, bool)> {
        let lv = self.lower_expr(left)?;
        let rv = self.lower_expr(right)?;

        match (lv.unsized_val, rv.unsized_val) {
            (None, Some(v)) => {
                self.check_unsized_fits(v, lv.width, lv.signed, right.span)?;
                let re = self.module.c_u64(lv.width, v);
                Ok((lv.expr, re, lv.width, lv.signed))
            }
            (Some(v), None) => {
                self.check_unsized_fits(v, rv.width, rv.signed, left.span)?;
                let le = self.module.c_u64(rv.width, v);
                Ok((le, rv.expr, rv.width, rv.signed))
            }
            (Some(va), Some(vb)) => {
                let w = lv.width.max(rv.width);
                let le = self.module.c_u64(w, va);
                let re = self.module.c_u64(w, vb);
                Ok((le, re, w, false))
            }
            (None, None) => {
                if lv.signed != rv.signed {
                    bail!(
                        "mixed signed and unsigned operands at {span} (left is {}, right is {}); wrap explicitly with `signed(...)` or `unsigned(...)`",
                        if lv.signed { "signed" } else { "unsigned" },
                        if rv.signed { "signed" } else { "unsigned" }
                    );
                }
                if lv.width != rv.width {
                    bail!(
                        "width mismatch at {span} ({}-bit vs {}-bit); use explicit `resize(expr, W)` or slice `[hi:lo]`",
                        lv.width,
                        rv.width
                    );
                }
                Ok((lv.expr, rv.expr, lv.width, lv.signed))
            }
        }
    }

    fn check_unsized_fits(&self, val: u64, width: u32, signed: bool, span: Span) -> Result<()> {
        if signed {
            if width < 64 {
                let max_pos = (1u64 << (width - 1)) - 1;
                if val > max_pos {
                    bail!("literal {val} overflows signed {width}-bit range (max {max_pos}) at {span}");
                }
            }
        } else if width < 64 && val >= (1u64 << width) {
            bail!("literal {val} overflows unsigned {width}-bit width at {span}");
        }
        Ok(())
    }

    fn get_hist_valid(&mut self, depth: u32) -> ExprId {
        let d = depth as usize;
        while self.hist_valid.len() <= d {
            let prev = *self.hist_valid.last().unwrap();
            let next_stage = self.make_resettable_reg_1(prev);
            self.hist_valid.push(next_stage);
        }
        self.hist_valid[d]
    }

    fn add_aux_reg(&mut self, tag: &str, width: u32, init: Option<ConstBits>) -> SignalId {
        let name = format!("$dsl_{tag}_{}", self.aux_counter);
        self.aux_counter += 1;
        let sid = self.module.add_signal(name, width, false, SignalKind::Reg);
        if let Some(c) = init {
            self.module.signal_mut(sid).init = Some(c);
        }
        sid
    }

    fn make_plain_reg(&mut self, expr: ExprId) -> ExprId {
        let w = self.module.width(expr);
        let sid = self.add_aux_reg("d", w, None);
        self.module.ffs.push(Ff {
            signal: sid,
            next: expr,
            init: None,
            clock: None,
            async_reset: None,
        });
        self.module.sig_expr(sid)
    }

    fn make_resettable_reg_1(&mut self, expr: ExprId) -> ExprId {
        let sid = self.add_aux_reg("r", 1, Some(ConstBits::zero(1)));
        let zero = self.module.c_bool(false);
        let next = self.module.mux(self.rst_expr, zero, expr);
        self.module.ffs.push(Ff {
            signal: sid,
            next,
            init: Some(ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });
        self.module.sig_expr(sid)
    }
}

fn collect_at_triggers(expr: &Expr, out: &mut Vec<Expr>) {
    match &expr.kind {
        ExprKind::Call { name, args } if name == "at_trigger" => {
            if let Some(a) = args.first() {
                out.push(a.clone());
            }
        }
        ExprKind::Unary { expr: e, .. } | ExprKind::Slice { expr: e, .. } => {
            collect_at_triggers(e, out)
        }
        ExprKind::Binary { left, right, .. } => {
            collect_at_triggers(left, out);
            collect_at_triggers(right, out);
        }
        ExprKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            collect_at_triggers(cond, out);
            collect_at_triggers(then_expr, out);
            collect_at_triggers(else_expr, out);
        }
        ExprKind::Call { args, .. } | ExprKind::Concat(args) => {
            for a in args {
                collect_at_triggers(a, out);
            }
        }
        ExprKind::Bool(_) | ExprKind::Int(_) | ExprKind::SizedLit { .. } | ExprKind::Ident(_) => {}
    }
}

fn eval_const_i64(expr: &Expr, params: &HashMap<String, i64>) -> Result<i64> {
    match &expr.kind {
        ExprKind::Bool(b) => Ok(i64::from(*b)),
        ExprKind::Int(v) => Ok(*v as i64),
        ExprKind::SizedLit { bits, .. } => {
            Ok(u64::from_str_radix(bits, 2).unwrap_or(0) as i64)
        }
        ExprKind::Ident(name) => params
            .get(name)
            .copied()
            .ok_or_else(|| anyhow!("non-constant identifier '{name}' at {}", expr.span)),
        ExprKind::Unary { op, expr: e } => {
            let v = eval_const_i64(e, params)?;
            Ok(match op {
                UnOp::Neg => -v,
                UnOp::LogNot => i64::from(v == 0),
                UnOp::BitNot => !v,
            })
        }
        ExprKind::Binary { op, left, right } => {
            let l = eval_const_i64(left, params)?;
            let r = eval_const_i64(right, params)?;
            Ok(match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div => {
                    if r == 0 {
                        bail!("division by zero in constant expression at {}", expr.span);
                    }
                    l / r
                }
                BinOp::Mod => {
                    if r == 0 {
                        bail!("modulo by zero in constant expression at {}", expr.span);
                    }
                    l % r
                }
                BinOp::Eq => i64::from(l == r),
                BinOp::Ne => i64::from(l != r),
                BinOp::Lt => i64::from(l < r),
                BinOp::Le => i64::from(l <= r),
                BinOp::Gt => i64::from(l > r),
                BinOp::Ge => i64::from(l >= r),
                BinOp::LogAnd => i64::from(l != 0 && r != 0),
                BinOp::LogOr => i64::from(l != 0 || r != 0),
                BinOp::Implies => i64::from(l == 0 || r != 0),
                BinOp::BitAnd => l & r,
                BinOp::BitOr => l | r,
                BinOp::BitXor => l ^ r,
                BinOp::Shl => l << r,
                BinOp::Shr => l >> r,
            })
        }
        _ => bail!("expression at {} must be a static constant", expr.span),
    }
}
