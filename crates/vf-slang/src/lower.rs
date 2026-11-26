use crate::ast::{AssertionExpr, ElaboratedDesign, Expr, SeqRepetition, Stmt, TimingJson};
use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};
use vf_ir::{
    AsyncReset, ClockEdge, ClockSpec, CombAssign, ConstBits, ExprId, ExprKind, Ff, MemId, MemWrite,
    Module, Property, PropertyKind, SignalId, SignalKind,
};

/// Automatic instrumentation flags used by Questa-style Formal Apps (AutoCheck & CoverCheck).
#[derive(Debug, Clone, Default)]
pub struct InstrumentOptions {
    /// Emit `cover` goals for every `if`/`else`/`case` branch (CoverCheck).
    pub branch_covers: bool,
    /// Emit assertions for `unique` / `unique0` / `priority` case & if overlap/completeness (AutoCheck).
    pub unique_priority_checks: bool,
    /// Emit assertions for out-of-bounds memory/array index accesses (AutoCheck).
    pub oob_memory_checks: bool,
    /// Emit assertions for dynamic bit-select & indexed part-select out-of-bounds (AutoCheck).
    pub oob_bitselect_checks: bool,
    /// Emit assertions for divide/modulo by zero (AutoCheck).
    pub div_zero_checks: bool,
    /// Emit assertions for unsigned & signed arithmetic add/sub/mul overflow/underflow (AutoCheck).
    pub overflow_checks: bool,
    /// Emit assertions for out-of-range shift amounts (`rhs >= width(lhs)`) (AutoCheck).
    pub shift_overflow_checks: bool,
    /// Emit formal assertions for inferred latches in `always_comb` / combinational `always @(*)` (AutoCheck).
    pub latch_checks: bool,
    /// Emit formal assertions for implicit narrowing width truncation / silent data loss (AutoCheck).
    pub truncation_checks: bool,
    /// Track multi-driver bus contention (comb+comb, seq+seq, comb+seq) into `module.lints` (AutoCheck).
    pub multi_driver_checks: bool,
    /// Tolerate combinational feedback loops during lowering and record them into `module.lints` (AutoCheck).
    pub allow_comb_loops: bool,
}

struct Lowerer {
    module: Module,
    inst: InstrumentOptions,
    mem_lo: HashMap<MemId, i32>,
    init_state_sig: Option<SignalId>,
    past_counter: usize,
    any_counter: usize,
    prop_counter: usize,
    aux_ffs: Vec<Ff>,
    inits: HashMap<SignalId, ConstBits>,
    seq_drivers: HashMap<SignalId, (ExprId, Option<ClockSpec>, Option<AsyncReset>)>,
    comb_drivers: HashMap<SignalId, ExprId>,
    /// Bitmask of bits already driven by continuous assigns / combinational processes (for multi-driver lint).
    comb_bit_masks: HashMap<SignalId, (u128, bool)>,
    /// Signals already flagged as multi-driven (dedup).
    multi_driven_reported: HashSet<SignalId>,
    /// Local constant loop variables (e.g. `for (int i = 0; i < 8; i++)`).
    const_locals: HashMap<String, i64>,
    /// Local expression bindings (e.g. pure SV function parameters and return vars).
    local_exprs: HashMap<String, ExprId>,
    /// Current statement guard while lowering expressions (for OOB / div-zero / overflow checks).
    cur_guard: ExprId,
    /// Enclosing SystemVerilog block/statement label (e.g. `a_reachable: assert (...)`).
    cur_label: Option<String>,
    /// Active SVA `disable iff (cond)` condition (aborts in-flight multi-cycle sequences).
    cur_disable: Option<ExprId>,
}

impl Lowerer {
    fn new(name: &str, inst: InstrumentOptions) -> Self {
        let mut module = Module::new(name);
        let one = module.c_bool(true);
        Self {
            module,
            inst,
            mem_lo: HashMap::new(),
            init_state_sig: None,
            past_counter: 0,
            any_counter: 0,
            prop_counter: 0,
            aux_ffs: Vec::new(),
            inits: HashMap::new(),
            seq_drivers: HashMap::new(),
            comb_drivers: HashMap::new(),
            comb_bit_masks: HashMap::new(),
            multi_driven_reported: HashSet::new(),
            const_locals: HashMap::new(),
            local_exprs: HashMap::new(),
            cur_guard: one,
            cur_label: None,
            cur_disable: None,
        }
    }

    fn lower_design(mut self, design: &ElaboratedDesign) -> Result<Module> {
        // 1. Register all signals and memories (honoring Yosys formal attributes!).
        for s in &design.signals {
            let mut kind = match s.direction.as_str() {
                "input" => SignalKind::Input,
                "output" => SignalKind::Output,
                _ => SignalKind::Wire,
            };
            let mut is_gclk = false;
            for attr in &s.attrs {
                match attr.as_str() {
                    "anyseq" | "allseq" => kind = SignalKind::AnySeq,
                    "anyconst" | "allconst" => kind = SignalKind::AnyConst,
                    "gclk" => is_gclk = true,
                    _ => {}
                }
            }
            let id = self.module.add_signal(&s.name, s.width, s.signed, kind);
            if is_gclk {
                self.module.signal_mut(id).is_clock = true;
            }
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

        // 2. Lower continuous assignments (`assign`, net initializers, flattened port bindings).
        let empty_env = HashMap::new();
        for ca in &design.assigns {
            let rhs = self.lower_expr(&ca.rhs, &empty_env)?;
            self.lower_comb_lhs(&ca.lhs, rhs)?;
        }

        // 3. Lower procedural blocks (`initial`, `always_comb`, `always_latch`, `always_ff`, `always`).
        for proc in &design.processes {
            match proc.kind.as_str() {
                "initial" => self.lower_initial(&proc.body)?,
                "always_comb" => self.lower_comb_proc(&proc.body, false)?,
                "always_latch" => self.lower_comb_proc(&proc.body, true)?,
                "always_ff" => self.lower_seq_proc(&proc.body)?,
                "always" => self.lower_always(&proc.body)?,
                _ => {}
            }
        }

        // 4. Finalize signal kinds, FFs, combinational assigns, and init values.
        for (&sig, init) in &self.inits {
            self.module.signal_mut(sig).init = Some(init.clone());
        }

        for (sig, (next, clock, async_reset)) in std::mem::take(&mut self.seq_drivers) {
            let s = self.module.signal_mut(sig);
            if s.kind == SignalKind::Wire {
                s.kind = SignalKind::Reg;
            }
            let init = self.inits.get(&sig).cloned();
            self.module.ffs.push(Ff {
                signal: sig,
                next,
                init,
                clock,
                async_reset,
            });
        }

        for ff in std::mem::take(&mut self.aux_ffs) {
            self.module.ffs.push(ff);
        }

        let ff_sigs: HashSet<SignalId> = self.module.ffs.iter().map(|f| f.signal).collect();
        for (sig, val) in std::mem::take(&mut self.comb_drivers) {
            if ff_sigs.contains(&sig) {
                if self.inst.multi_driver_checks {
                    self.record_multi_driven(
                        sig,
                        "driven by both combinational and sequential logic",
                    );
                }
            } else {
                self.module.comb.push(CombAssign {
                    signal: sig,
                    value: val,
                });
            }
        }

        vf_ir::opt::resolve_self_slices(&mut self.module);
        if self.inst.allow_comb_loops {
            let loops = self.module.break_comb_loops();
            for cyc in loops {
                let sig = cyc.first().cloned().unwrap_or_default();
                let path = cyc.join(" -> ");
                self.module.lints.push(vf_ir::StructuralLint {
                    category: "comb_loop".into(),
                    signal: sig,
                    message: format!("combinational feedback loop: {path}"),
                    is_bug: true,
                });
            }
        }
        self.module
            .topo_sort_comb()
            .map_err(|e| anyhow!(e))?;
        vf_ir::opt::optimize(&mut self.module);
        Ok(self.module)
    }

    fn record_multi_driven(&mut self, sig: SignalId, reason: &str) {
        let name = self.module.signal(sig).name.clone();
        if name.starts_with('$') {
            return;
        }
        if self.multi_driven_reported.insert(sig) {
            self.module.lints.push(vf_ir::StructuralLint {
                category: "multi_driven".into(),
                signal: name.clone(),
                message: format!("signal '{name}' has multiple drivers ({reason})"),
                is_bug: true,
            });
        }
    }

    fn record_comb_driver_bits(&mut self, sig: SignalId, hi: u32, lo: u32, reason: &str) {
        if !self.inst.multi_driver_checks {
            return;
        }
        let width = self.module.signal(sig).width;
        let is_wide = width > 128 || hi >= 128;
        let bit_mask: u128 = if is_wide {
            u128::MAX
        } else {
            let span = hi - lo + 1;
            let raw = if span >= 128 {
                u128::MAX
            } else {
                (1u128 << span) - 1
            };
            raw << lo
        };
        let entry = self.comb_bit_masks.entry(sig).or_insert((0, false));
        let overlap = if is_wide || entry.1 {
            entry.0 != 0 || entry.1
        } else {
            (entry.0 & bit_mask) != 0
        };
        entry.0 |= bit_mask;
        entry.1 |= is_wide;
        if overlap {
            self.record_multi_driven(sig, reason);
        }
    }

    fn lower_comb_lhs(&mut self, lhs: &Expr, rhs: ExprId) -> Result<()> {
        match lhs {
            Expr::Ref { name, width, .. } => {
                let sig = self
                    .module
                    .find_signal(name)
                    .ok_or_else(|| anyhow!("unknown signal in LHS: {name}"))?;
                let val = self.module.resize(rhs, *width, false);
                self.record_comb_driver_bits(sig, width.saturating_sub(1), 0, "overlapping combinational drivers");
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
            Expr::FieldSelect {
                value,
                bit_offset,
                width,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        let lo = *bit_offset as u32;
                        let hi = lo + *width - 1;
                        self.record_comb_driver_bits(sig, hi, lo, "overlapping field assignments");
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
                bail!("unsupported packed struct field LHS in continuous assignment")
            }
            Expr::BitSelect {
                width,
                value,
                selector,
                range_left,
                range_right,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        if let Some(idx) =
                            self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64())
                        {
                            let elem_idx =
                                self.normalize_bit_index(idx as i64, *range_left, *range_right);
                            let elem_w = (*width).max(1);
                            let lo = elem_idx * elem_w;
                            let hi = lo + elem_w - 1;
                            self.record_comb_driver_bits(sig, hi, lo, "overlapping bit assignments");
                            let base = self
                                .comb_drivers
                                .get(&sig)
                                .copied()
                                .unwrap_or_else(|| {
                                    self.module.c_u64(self.module.signal(sig).width, 0)
                                });
                            let updated = self.module.update_slice(base, hi, lo, rhs);
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
                        self.record_comb_driver_bits(sig, hi, lo, "overlapping slice assignments");
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
            Stmt::ForLoop {
                loop_var,
                init,
                stop,
                step,
                body,
                ..
            } => {
                if let (Some(var), Some(init_e), Some(stop_e), Some(step_e), Some(body_s)) =
                    (loop_var, init, stop, step, body)
                {
                    let start = self
                        .try_eval_i64(init_e)
                        .ok_or_else(|| anyhow!("non-constant initial for-loop bound"))?;
                    let prev = self.const_locals.insert(var.clone(), start);
                    for _ in 0..4096 {
                        if self.try_eval_i64(stop_e).unwrap_or(0) == 0 {
                            break;
                        }
                        self.lower_initial(body_s)?;
                        let next_val = self.eval_loop_step(var, step_e)?;
                        self.const_locals.insert(var.clone(), next_val);
                    }
                    if let Some(p) = prev {
                        self.const_locals.insert(var.clone(), p);
                    } else {
                        self.const_locals.remove(var);
                    }
                }
                Ok(())
            }
            Stmt::Assign { lhs, rhs } | Stmt::Nba { lhs, rhs } => {
                match lhs {
                    Expr::Ref {
                        name,
                        width,
                        signed,
                    } => {
                        if let Some(sig) = self.module.find_signal(name) {
                            if let Some(c) = self.try_eval_const(rhs, *width, *signed) {
                                self.inits.insert(sig, c);
                            }
                        }
                    }
                    Expr::MemSelect {
                        value, selector, ..
                    } => {
                        if let Expr::Ref { name, .. } = value.as_ref() {
                            if let Some(&mem_id) = self.module.mem_by_name.get(name) {
                                let dw = self.module.memory(mem_id).data_width;
                                if let (Some(addr), Some(val)) = (
                                    self.try_eval_const(selector, 32, false)
                                        .and_then(|c| c.to_u64()),
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
            match timing.as_ref() {
                TimingJson::Implicit => self.lower_comb_proc(body, false),
                TimingJson::SignalEvent { edge, expr } => {
                    if edge == "none" {
                        self.lower_comb_proc(body, false)
                    } else {
                        let clk = self.extract_clock_spec(edge, expr);
                        self.lower_clocked_body(body, clk, None)
                    }
                }
                TimingJson::EventList { events } => {
                    let has_edge = events.iter().any(|e| {
                        matches!(e, TimingJson::SignalEvent { edge, .. } if edge != "none")
                    });
                    if !has_edge {
                        self.lower_comb_proc(body, false)
                    } else {
                        let (clk, async_rst) = self.extract_clock_and_async_reset(events);
                        self.lower_clocked_body(body, clk, async_rst)
                    }
                }
                TimingJson::Other => self.lower_comb_proc(body, false),
            }
        } else {
            self.lower_comb_proc(stmt, false)
        }
    }

    fn lower_seq_proc(&mut self, stmt: &Stmt) -> Result<()> {
        if let Stmt::Timed { timing, body } = stmt {
            let (clk, async_rst) = match timing.as_ref() {
                TimingJson::SignalEvent { edge, expr } => (self.extract_clock_spec(edge, expr), None),
                TimingJson::EventList { events } => self.extract_clock_and_async_reset(events),
                _ => (None, None),
            };
            self.lower_clocked_body(body, clk, async_rst)
        } else {
            self.lower_clocked_body(stmt, None, None)
        }
    }

    fn extract_clock_spec(&mut self, edge: &str, expr: &Expr) -> Option<ClockSpec> {
        if let Expr::Ref { name, .. } = expr {
            if let Some(sig) = self.module.find_signal(name) {
                self.module.signal_mut(sig).is_clock = true;
                let e = if edge == "negedge" {
                    ClockEdge::NegEdge
                } else {
                    ClockEdge::PosEdge
                };
                return Some(ClockSpec { signal: sig, edge: e });
            }
        }
        None
    }

    fn extract_clock_and_async_reset(
        &mut self,
        events: &[TimingJson],
    ) -> (Option<ClockSpec>, Option<(SignalId, bool)>) {
        let mut clk = None;
        let mut async_rst = None;
        for (idx, ev) in events.iter().enumerate() {
            if let TimingJson::SignalEvent { edge, expr } = ev {
                if idx == 0 {
                    clk = self.extract_clock_spec(edge, expr);
                } else if let Expr::Ref { name, .. } = expr {
                    if let Some(sig) = self.module.find_signal(name) {
                        let active_low = edge == "negedge";
                        async_rst = Some((sig, active_low));
                    }
                }
            }
        }
        (clk, async_rst)
    }

    fn lower_comb_proc(&mut self, stmt: &Stmt, is_latch: bool) -> Result<()> {
        let mut env: HashMap<SignalId, ExprId> = HashMap::new();
        let mut nba_writes: HashMap<SignalId, ExprId> = HashMap::new();
        let mut mem_writes: Vec<MemWrite> = Vec::new();
        let en = self.module.c_bool(true);
        self.lower_stmt(stmt, en, &mut env, &mut nba_writes, &mut mem_writes, false, None)?;
        for (sig, val) in nba_writes {
            env.entry(sig).or_insert(val);
        }

        // Deterministic signal order for reproducible property names and IR ids.
        let mut entries: Vec<(SignalId, ExprId)> = env.into_iter().collect();
        entries.sort_by_key(|(s, _)| s.0);

        for (sig, raw_val) in entries {
            let w = self.module.signal(sig).width;
            let val = collapse_slice_concat(&mut self.module, raw_val);

            // Detect latch-style self-retention (`Mux(en, new_val, Signal(sig))`) vs full assignment.
            let final_val = if let Some(latch_cond) = unassigned_path_cond(&mut self.module, val, sig) {
                let sname = self.module.signal(sig).name.clone();
                if self.inst.latch_checks && !is_latch && !sname.starts_with('$') {
                    let no_latch = self.module.logic_not(latch_cond);
                    let idx = self.prop_counter;
                    self.prop_counter += 1;
                    let one = self.module.c_bool(true);
                    self.add_named_property(
                        PropertyKind::Assert,
                        format!("autocheck_inferred_latch_{sname}_{idx}"),
                        no_latch,
                        one,
                        "",
                    );
                }
                if is_latch || self.inst.latch_checks || self.inst.allow_comb_loops {
                    // Model the latch hold state via an auxiliary FF so `topo_sort_comb()` succeeds
                    // and the formal solver can prove whether `latch_cond` is reachable or unreachable.
                    let hold_name = format!("$latch_hold_{sname}_{}", self.aux_ffs.len());
                    let hold_sig = self.module.add_signal(hold_name, w, false, SignalKind::Reg);
                    let hold_e = self.module.sig_expr(hold_sig);
                    let mut cache = HashMap::new();
                    let replaced = replace_sig_in_expr(&mut self.module, val, sig, hold_e, &mut cache);
                    self.aux_ffs.push(Ff {
                        signal: hold_sig,
                        next: replaced,
                        init: self.inits.get(&sig).cloned(),
                        clock: None,
                        async_reset: None,
                    });
                    replaced
                } else {
                    val
                }
            } else {
                val
            };

            self.record_comb_driver_bits(
                sig,
                w.saturating_sub(1),
                0,
                "overlapping combinational drivers",
            );
            self.comb_drivers.insert(sig, final_val);
        }
        Ok(())
    }

    fn lower_clocked_body(
        &mut self,
        stmt: &Stmt,
        clock: Option<ClockSpec>,
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
            clock,
        )?;

        // If the clocked block declared an async reset sensitivity (e.g. `posedge clk or negedge rst_n`),
        // extract the reset condition + reset value from the top-level `if` when present so `async2sync`
        // can model immediate asynchronous reset accurately.
        let mut arst_vals: HashMap<SignalId, AsyncReset> = HashMap::new();
        if let Some((rst_sig, active_low)) = async_rst {
            let rst_e = self.module.sig_expr(rst_sig);
            let rst_cond = if active_low {
                self.module.logic_not(rst_e)
            } else {
                self.module.to_bool(rst_e)
            };
            if let Some(Stmt::If { then, .. }) = unwrap_single_stmt(stmt) {
                let mut rst_env = HashMap::new();
                let mut rst_nba = HashMap::new();
                let mut dummy_mw = Vec::new();
                let _ = self.lower_stmt(
                    then,
                    rst_cond,
                    &mut rst_env,
                    &mut rst_nba,
                    &mut dummy_mw,
                    true,
                    clock,
                );
                for (sig, val) in rst_nba.into_iter().chain(rst_env) {
                    arst_vals.insert(
                        sig,
                        AsyncReset {
                            cond: rst_cond,
                            reset_val: val,
                        },
                    );
                }
            }
        }

        let mut driven_in_this_block: HashSet<SignalId> = HashSet::new();
        for (sig, val) in nba_writes {
            let arst = arst_vals.remove(&sig);
            let combined = if let Some(blk_val) = blocking_env.remove(&sig) {
                let mut cache = HashMap::new();
                replace_sig_in_expr(&mut self.module, val, sig, blk_val, &mut cache)
            } else {
                val
            };
            if self.inst.multi_driver_checks && self.seq_drivers.contains_key(&sig) {
                self.record_multi_driven(sig, "driven by multiple clocked always blocks");
            }
            driven_in_this_block.insert(sig);
            self.seq_drivers.insert(sig, (combined, clock, arst));
        }
        for (sig, val) in blocking_env {
            let arst = arst_vals.remove(&sig);
            if self.inst.multi_driver_checks
                && !driven_in_this_block.contains(&sig)
                && self.seq_drivers.contains_key(&sig)
            {
                self.record_multi_driven(sig, "driven by multiple clocked always blocks");
            }
            self.seq_drivers.entry(sig).or_insert((val, clock, arst));
        }
        self.module.mem_writes.extend(mem_writes);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        guard: ExprId,
        env: &mut HashMap<SignalId, ExprId>,
        nba: &mut HashMap<SignalId, ExprId>,
        mem_writes: &mut Vec<MemWrite>,
        is_clocked: bool,
        clock: Option<ClockSpec>,
    ) -> Result<()> {
        match stmt {
            Stmt::Empty => Ok(()),
            Stmt::Block { label, items } => {
                let saved_label = self.cur_label.clone();
                if let Some(l) = label.as_ref().filter(|s| !s.is_empty()) {
                    self.cur_label = Some(l.clone());
                }
                for item in items {
                    self.lower_stmt(item, guard, env, nba, mem_writes, is_clocked, clock)?;
                }
                self.cur_label = saved_label;
                Ok(())
            }
            Stmt::Timed { body, .. } => {
                self.lower_stmt(body, guard, env, nba, mem_writes, is_clocked, clock)
            }
            Stmt::Assign { lhs, rhs } => {
                let val = self.lower_expr(rhs, env)?;
                self.apply_write(lhs, val, guard, env, mem_writes, false, clock)
            }
            Stmt::Nba { lhs, rhs } => {
                let val = self.lower_expr(rhs, env)?;
                self.apply_write(lhs, val, guard, nba, mem_writes, true, clock)
            }
            Stmt::If {
                check,
                loc,
                cond,
                then,
                else_branch,
            } => {
                let saved_g = self.cur_guard;
                self.cur_guard = guard;
                let cond_raw = self.lower_expr(cond, env)?;
                let cond_b = self.module.to_bool(cond_raw);
                let not_cond = self.module.logic_not(cond_b);
                let then_guard = self.module.logic_and(guard, cond_b);
                let else_guard = self.module.logic_and(guard, not_cond);

                if self.inst.branch_covers {
                    let one = self.module.c_bool(true);
                    let tag = if loc.is_empty() {
                        format!("branch_if_then_{}", self.prop_counter)
                    } else {
                        format!("branch_if_then@{loc}")
                    };
                    self.add_named_property(PropertyKind::Cover, tag, then_guard, one, loc);
                    let etag = if loc.is_empty() {
                        format!("branch_if_else_{}", self.prop_counter)
                    } else {
                        format!("branch_if_else@{loc}")
                    };
                    self.add_named_property(PropertyKind::Cover, etag, else_guard, one, loc);
                }

                if self.inst.unique_priority_checks
                    && (check == "Unique" || check == "Priority")
                    && else_branch.is_none()
                {
                    let tag = format!("autocheck_{check}_if_complete@{loc}");
                    self.add_named_property(PropertyKind::Assert, tag, cond_b, guard, loc);
                }

                let mut then_env = env.clone();
                let mut then_nba = nba.clone();
                self.cur_guard = then_guard;
                self.lower_stmt(
                    then,
                    then_guard,
                    &mut then_env,
                    &mut then_nba,
                    mem_writes,
                    is_clocked,
                    clock,
                )?;

                let mut else_env = env.clone();
                let mut else_nba = nba.clone();
                if let Some(eb) = else_branch {
                    self.cur_guard = else_guard;
                    self.lower_stmt(
                        eb,
                        else_guard,
                        &mut else_env,
                        &mut else_nba,
                        mem_writes,
                        is_clocked,
                        clock,
                    )?;
                }
                self.cur_guard = saved_g;

                self.merge_maps(cond_b, env, &then_env, &else_env);
                self.merge_maps(cond_b, nba, &then_nba, &else_nba);
                Ok(())
            }
            Stmt::Case {
                check,
                loc,
                case_kind,
                expr,
                items,
                default,
            } => {
                let saved_g = self.cur_guard;
                self.cur_guard = guard;
                let sel = self.lower_expr(expr, env)?;
                let sel_signed = expr.is_signed();

                let base_env = env.clone();
                let base_nba = nba.clone();

                let mut branch_results: Vec<(
                    ExprId,
                    HashMap<SignalId, ExprId>,
                    HashMap<SignalId, ExprId>,
                )> = Vec::new();
                let mut unmatched = guard;
                let mut any_match = self.module.c_bool(false);
                let mut overlap = self.module.c_bool(false);

                for (idx, item) in items.iter().enumerate() {
                    let mut item_match = self.module.c_bool(false);
                    for e in &item.exprs {
                        let eq = self.lower_case_match(case_kind, sel, sel_signed, e, &base_env)?;
                        item_match = self.module.logic_or(item_match, eq);
                    }
                    let dup = self.module.logic_and(any_match, item_match);
                    overlap = self.module.logic_or(overlap, dup);
                    any_match = self.module.logic_or(any_match, item_match);

                    let branch_guard = self.module.logic_and(unmatched, item_match);
                    let not_match = self.module.logic_not(item_match);
                    unmatched = self.module.logic_and(unmatched, not_match);

                    if self.inst.branch_covers {
                        let one = self.module.c_bool(true);
                        let tag = format!("branch_case_item_{idx}@{loc}");
                        self.add_named_property(PropertyKind::Cover, tag, branch_guard, one, loc);
                    }

                    let mut b_env = base_env.clone();
                    let mut b_nba = base_nba.clone();
                    self.cur_guard = branch_guard;
                    self.lower_stmt(
                        &item.body,
                        branch_guard,
                        &mut b_env,
                        &mut b_nba,
                        mem_writes,
                        is_clocked,
                        clock,
                    )?;
                    branch_results.push((item_match, b_env, b_nba));
                }

                if self.inst.unique_priority_checks {
                    if check == "Unique" || check == "Unique0" {
                        let no_overlap = self.module.logic_not(overlap);
                        let tag = format!("autocheck_{check}_case_overlap@{loc}");
                        self.add_named_property(PropertyKind::Assert, tag, no_overlap, guard, loc);
                    }
                    if (check == "Unique" || check == "Priority") && default.is_none() {
                        let tag = format!("autocheck_{check}_case_complete@{loc}");
                        self.add_named_property(PropertyKind::Assert, tag, any_match, guard, loc);
                    }
                }

                let mut cur_env = base_env.clone();
                let mut cur_nba = base_nba.clone();
                if let Some(def) = default {
                    if self.inst.branch_covers {
                        let one = self.module.c_bool(true);
                        let tag = format!("branch_case_default@{loc}");
                        self.add_named_property(PropertyKind::Cover, tag, unmatched, one, loc);
                    }
                    self.cur_guard = unmatched;
                    self.lower_stmt(
                        def,
                        unmatched,
                        &mut cur_env,
                        &mut cur_nba,
                        mem_writes,
                        is_clocked,
                        clock,
                    )?;
                } else if !branch_results.is_empty()
                    && (case_kind == "normal" || case_kind == "case")
                {
                    // Check if constant case items statically cover all 2^sel_w values.
                    let sel_w = self.module.width(sel);
                    if sel_w <= 10 {
                        let mut covered = std::collections::HashSet::new();
                        for item in items {
                            for e in &item.exprs {
                                if let Some(c) = self.try_eval_const(e, sel_w, sel_signed) {
                                    if let Some(v) = c.to_u64() {
                                        covered.insert(v);
                                    }
                                }
                            }
                        }
                        if covered.len() == (1usize << sel_w) {
                            let (_, last_env, last_nba) = branch_results.pop().unwrap();
                            cur_env = last_env;
                            cur_nba = last_nba;
                        }
                    }
                }
                self.cur_guard = saved_g;

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
            Stmt::ForLoop {
                loc,
                loop_var,
                init,
                stop,
                step,
                body,
            } => {
                let (Some(var), Some(init_e), Some(stop_e), Some(step_e), Some(body_s)) =
                    (loop_var, init, stop, step, body)
                else {
                    bail!("procedural for-loop at {loc} could not be statically unrolled");
                };
                let start = self
                    .try_eval_i64(init_e)
                    .ok_or_else(|| anyhow!("non-constant for-loop initializer at {loc}"))?;
                let prev = self.const_locals.insert(var.clone(), start);
                let mut iters = 0usize;
                loop {
                    if self.try_eval_i64(stop_e).unwrap_or(0) == 0 {
                        break;
                    }
                    iters += 1;
                    if iters > 4096 {
                        bail!("procedural for-loop at {loc} exceeded 4096-iteration unroll limit");
                    }
                    self.lower_stmt(body_s, guard, env, nba, mem_writes, is_clocked, clock)?;
                    let next_val = self.eval_loop_step(var, step_e)?;
                    self.const_locals.insert(var.clone(), next_val);
                }
                if let Some(p) = prev {
                    self.const_locals.insert(var.clone(), p);
                } else {
                    self.const_locals.remove(var);
                }
                Ok(())
            }
            Stmt::RepeatLoop { count, body } => {
                for _ in 0..(*count).min(4096) {
                    self.lower_stmt(body, guard, env, nba, mem_writes, is_clocked, clock)?;
                }
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
            Stmt::Return { expr } => {
                if let Some(e) = expr {
                    let v = self.lower_expr(e, env)?;
                    self.local_exprs.insert("$return".to_string(), v);
                }
                Ok(())
            }
            Stmt::ExprStmt { .. } => Ok(()),
            Stmt::Unsupported { loc } => {
                bail!("unsupported SystemVerilog statement at {loc}")
            }
        }
    }

    fn lower_case_match(
        &mut self,
        case_kind: &str,
        sel: ExprId,
        sel_signed: bool,
        pat: &Expr,
        env: &HashMap<SignalId, ExprId>,
    ) -> Result<ExprId> {
        if let Expr::ValueRange { left, right, .. } = pat {
            let lo = self.lower_expr(left, env)?;
            let hi = self.lower_expr(right, env)?;
            let both_signed = sel_signed && left.is_signed() && right.is_signed();
            let w = self
                .module
                .width(sel)
                .max(self.module.width(lo))
                .max(self.module.width(hi));
            let s = self.module.resize(sel, w, both_signed);
            let l = self.module.resize(lo, w, both_signed);
            let h = self.module.resize(hi, w, both_signed);
            let ge = self.module.intern(
                1,
                if both_signed {
                    ExprKind::Sle(l, s)
                } else {
                    ExprKind::Ule(l, s)
                },
            );
            let le = self.module.intern(
                1,
                if both_signed {
                    ExprKind::Sle(s, h)
                } else {
                    ExprKind::Ule(s, h)
                },
            );
            return Ok(self.module.logic_and(ge, le));
        }
        if case_kind == "casez" || case_kind == "casex" || case_kind == "inside" {
            if let Some((bits, mask)) = extract_wildcard_const(pat, case_kind == "casex" || case_kind == "inside") {
                let w = self.module.width(sel).max(bits.width);
                let sel_w = self.module.resize(sel, w, sel_signed);
                let val_c = self.module.c_bits(bits.resize_zext(w));
                let mask_c = self.module.c_bits(mask.resize_zext(w));
                let masked_sel = self.module.intern(w, ExprKind::And(sel_w, mask_c));
                let masked_val = self.module.intern(w, ExprKind::And(val_c, mask_c));
                return Ok(self.module.intern(1, ExprKind::Eq(masked_sel, masked_val)));
            }
        }
        let ev = self.lower_expr(pat, env)?;
        Ok(self.module.eq(sel, ev, sel_signed))
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
                if let TimingJson::SignalEvent { edge, expr: clk } = clocking {
                    let _ = self.extract_clock_spec(edge, clk);
                }
                self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
            }
            AssertionExpr::StrongWeak { expr } => {
                self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
            }
            AssertionExpr::DisableIff { cond, expr } => {
                let dis = self.lower_expr(cond, env)?;
                let dis_b = self.module.to_bool(dis);
                let prev_dis = self.cur_disable;
                self.cur_disable = Some(match prev_dis {
                    Some(p) => self.module.logic_or(p, dis_b),
                    None => dis_b,
                });
                let not_dis = self.module.logic_not(dis_b);
                let new_guard = self.module.logic_and(guard, not_dis);
                let res = self.lower_concurrent_assert(assert_kind, expr, new_guard, env, loc);
                self.cur_disable = prev_dis;
                res
            }
            AssertionExpr::Simple { expr, repetition } => {
                let c = self.lower_expr(expr, env)?;
                let c = self.module.to_bool(c);
                if let Some(rep) = repetition {
                    self.lower_consequent_repetition(assert_kind, c, rep, guard, loc)?;
                } else {
                    let en = self.apply_disable(guard);
                    self.add_property(assert_kind, c, en, loc);
                }
                Ok(())
            }
            AssertionExpr::SeqWithMatch { expr, repetition } => {
                if let Some(rep) = repetition {
                    let c = self.lower_assertion_bool(expr, env)?;
                    self.lower_consequent_repetition(assert_kind, c, rep, guard, loc)
                } else {
                    self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
                }
            }
            AssertionExpr::FirstMatch { expr } => {
                self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
            }
            AssertionExpr::Conditional {
                cond,
                if_expr,
                else_expr,
            } => {
                let c = self.lower_expr(cond, env)?;
                let cb = self.module.to_bool(c);
                let g_then = self.module.logic_and(guard, cb);
                self.lower_concurrent_assert(assert_kind, if_expr, g_then, env, loc)?;
                if let Some(ee) = else_expr {
                    let ncb = self.module.logic_not(cb);
                    let g_else = self.module.logic_and(guard, ncb);
                    self.lower_concurrent_assert(assert_kind, ee, g_else, env, loc)?;
                }
                Ok(())
            }
            AssertionExpr::Unary { op, expr } => match op.as_str() {
                "Always" | "SAlways" => {
                    self.lower_concurrent_assert(assert_kind, expr, guard, env, loc)
                }
                "Eventually" | "SEventually" => {
                    let c = self.lower_assertion_bool(expr, env)?;
                    let mapped_kind = match assert_kind {
                        "assume" => "fair",
                        _ => "live",
                    };
                    let en = self.apply_disable(guard);
                    self.add_property(mapped_kind, c, en, loc);
                    Ok(())
                }
                "Not" => {
                    let inner = self.lower_assertion_bool(expr, env)?;
                    let negated = self.module.logic_not(inner);
                    let en = self.apply_disable(guard);
                    self.add_property(assert_kind, negated, en, loc);
                    Ok(())
                }
                other => bail!("unsupported SVA unary operator '{other}' at {loc}"),
            },
            AssertionExpr::Binary { op, left, right } => match op.as_str() {
                "OverlappedImplication" | "Implies" => {
                    let a = self.lower_assertion_bool(left, env)?;
                    let new_guard = self.module.logic_and(guard, a);
                    self.lower_concurrent_assert(assert_kind, right, new_guard, env, loc)
                }
                "NonOverlappedImplication" => {
                    let a = self.lower_assertion_bool(left, env)?;
                    let trig = self.module.logic_and(guard, a);
                    let prev_trig = self.make_sva_delay_reg(trig);
                    self.lower_concurrent_assert(assert_kind, right, prev_trig, env, loc)
                }
                "And" => {
                    let a = self.lower_assertion_bool(left, env)?;
                    let b = self.lower_assertion_bool(right, env)?;
                    let both = self.module.logic_and(a, b);
                    let en = self.apply_disable(guard);
                    self.add_property(assert_kind, both, en, loc);
                    Ok(())
                }
                "Or" => {
                    let a = self.lower_assertion_bool(left, env)?;
                    let b = self.lower_assertion_bool(right, env)?;
                    let either = self.module.logic_or(a, b);
                    let en = self.apply_disable(guard);
                    self.add_property(assert_kind, either, en, loc);
                    Ok(())
                }
                "Iff" => {
                    let a = self.lower_assertion_bool(left, env)?;
                    let b = self.lower_assertion_bool(right, env)?;
                    let eq = self.module.eq(a, b, false);
                    let en = self.apply_disable(guard);
                    self.add_property(assert_kind, eq, en, loc);
                    Ok(())
                }
                "Until" | "SUntil" | "UntilWith" | "SUntilWith" => {
                    let p = self.lower_assertion_bool(left, env)?;
                    let q = self.lower_assertion_bool(right, env)?;
                    let strong = op == "SUntil" || op == "SUntilWith";
                    let inclusive = op == "UntilWith" || op == "SUntilWith";
                    self.lower_sva_until(assert_kind, p, q, guard, strong, inclusive, loc);
                    Ok(())
                }
                "Throughout" => {
                    // `cond throughout seq`: `cond` holds on every cycle while `seq` evaluates.
                    let c = self.lower_assertion_bool(left, env)?;
                    let not_c = self.module.logic_not(c);
                    // For `cond throughout (##[M:N] target)` or simple seq, lower right AND assert `cond` until seq finishes.
                    let right_b = self.lower_assertion_bool(right, env)?;
                    let en = self.apply_disable(guard);
                    let both = self.module.logic_and(c, right_b);
                    let _ = not_c;
                    self.add_property(assert_kind, both, en, loc);
                    Ok(())
                }
                other => bail!("unsupported SVA binary operator '{other}' at {loc}"),
            },
            AssertionExpr::SeqConcat { elements } => {
                // Lower sequence `##d0 e0 ##d1 e1 ...` (including `##2 ack`, `##[1:3] done`, and unbounded `##[1:$] ack`).
                let mut cur_trig = guard;
                for (i, el) in elements.iter().enumerate() {
                    let d_min = el.delay_min as usize;
                    let is_unbounded = el.delay_max.is_none();
                    let d_max = el.delay_max.unwrap_or(el.delay_min) as usize;
                    let is_last = i + 1 == elements.len();

                    // Unbounded SVA range delay (`##[d_min:$] target`):
                    if is_unbounded {
                        for _ in 0..d_min {
                            cur_trig = self.make_sva_delay_reg(cur_trig);
                        }
                        let target = self.lower_assertion_bool(&el.expr, env)?;
                        if is_last && assert_kind == "cover" {
                            // Sticky armed latch: cover hits if `target` holds at ANY cycle >= d_min after trigger.
                            let arm_name = format!("$sva_unb_cov_{}", self.past_counter);
                            self.past_counter += 1;
                            let arm_sig = self.module.add_signal(arm_name, 1, false, SignalKind::Reg);
                            self.module.signal_mut(arm_sig).init = Some(ConstBits::zero(1));
                            let arm_e = self.module.sig_expr(arm_sig);
                            let armed_now = self.module.logic_or(arm_e, cur_trig);
                            let next_arm = self.apply_disable(armed_now);
                            self.aux_ffs.push(Ff {
                                signal: arm_sig,
                                next: next_arm,
                                init: Some(ConstBits::zero(1)),
                                clock: None,
                                async_reset: None,
                            });
                            let en = self.apply_disable(armed_now);
                            self.add_property("cover", target, en, loc);
                            return Ok(());
                        } else if is_last {
                            // Assert/Assume `a |-> ##[d_min:$] b`: strong eventual response (liveness/fairness)!
                            let pend_name = format!("$sva_unb_pend_{}", self.past_counter);
                            self.past_counter += 1;
                            let pend_sig = self.module.add_signal(pend_name, 1, false, SignalKind::Reg);
                            self.module.signal_mut(pend_sig).init = Some(ConstBits::zero(1));
                            let pend_e = self.module.sig_expr(pend_sig);
                            let active = self.module.logic_or(pend_e, cur_trig);
                            let not_target = self.module.logic_not(target);
                            let still_pend_raw = self.module.logic_and(active, not_target);
                            let still_pend = self.apply_disable(still_pend_raw);
                            self.aux_ffs.push(Ff {
                                signal: pend_sig,
                                next: still_pend,
                                init: Some(ConstBits::zero(1)),
                                clock: None,
                                async_reset: None,
                            });
                            let done_or_idle = self.module.logic_not(still_pend);
                            let one = self.module.c_bool(true);
                            let mapped = match assert_kind {
                                "assume" => "fair",
                                _ => "live",
                            };
                            self.add_property(mapped, done_or_idle, one, loc);
                            return Ok(());
                        } else {
                            // Intermediate `##[d_min:$] e_i` in a longer sequence: sticky arm until `e_i` fires.
                            let arm_name = format!("$sva_unb_mid_{}", self.past_counter);
                            self.past_counter += 1;
                            let arm_sig = self.module.add_signal(arm_name, 1, false, SignalKind::Reg);
                            self.module.signal_mut(arm_sig).init = Some(ConstBits::zero(1));
                            let arm_e = self.module.sig_expr(arm_sig);
                            let armed_now = self.module.logic_or(arm_e, cur_trig);
                            let next_arm = self.apply_disable(armed_now);
                            self.aux_ffs.push(Ff {
                                signal: arm_sig,
                                next: next_arm,
                                init: Some(ConstBits::zero(1)),
                                clock: None,
                                async_reset: None,
                            });
                            cur_trig = self.module.logic_and(armed_now, target);
                            continue;
                        }
                    }

                    // Consequent bounded range delay (`##[d_min:d_max] target`):
                    // In IEEE 1800 SVA, `assert (a |-> ##[M:N] b)` means `b` holds on AT LEAST ONE
                    // cycle in `[M..=N]` after `a` (not all cycles!).
                    if is_last && d_min < d_max && assert_kind != "cover" {
                        let target = self.lower_assertion_bool(&el.expr, env)?;
                        let mut prev_trig = cur_trig;
                        let mut prev_met = if d_min == 0 {
                            target
                        } else {
                            self.module.c_bool(false)
                        };
                        for slot in 1..=d_max {
                            let slot_trig = self.make_sva_delay_reg(prev_trig);
                            let slot_met = self.make_sva_delay_reg(prev_met);
                            let met_now = if slot >= d_min {
                                self.module.logic_or(slot_met, target)
                            } else {
                                slot_met
                            };
                            if slot == d_max {
                                let en = self.apply_disable(slot_trig);
                                self.add_property(assert_kind, met_now, en, loc);
                            } else {
                                prev_trig = slot_trig;
                                prev_met = met_now;
                            }
                        }
                        return Ok(());
                    }

                    if d_min > 0 || d_max > 0 {
                        if d_min == d_max {
                            for _ in 0..d_min {
                                cur_trig = self.make_sva_delay_reg(cur_trig);
                            }
                        } else {
                            let mut shifted = cur_trig;
                            let mut any = if d_min == 0 {
                                cur_trig
                            } else {
                                self.module.c_bool(false)
                            };
                            for step in 1..=d_max {
                                shifted = self.make_sva_delay_reg(shifted);
                                if step >= d_min {
                                    any = self.module.logic_or(any, shifted);
                                }
                            }
                            cur_trig = any;
                        }
                    }
                    if is_last {
                        self.lower_concurrent_assert(assert_kind, &el.expr, cur_trig, env, loc)?;
                    } else {
                        let cond = self.lower_assertion_bool(&el.expr, env)?;
                        cur_trig = self.module.logic_and(cur_trig, cond);
                    }
                }
                Ok(())
            }
            _ => bail!("unsupported concurrent assertion expression at {loc}"),
        }
    }

    fn lower_consequent_repetition(
        &mut self,
        assert_kind: &str,
        cond: ExprId,
        rep: &SeqRepetition,
        guard: ExprId,
        loc: &str,
    ) -> Result<()> {
        let min = rep.min as usize;
        match rep.kind.as_str() {
            "consecutive" => {
                if min == 0 {
                    return Ok(());
                }
                let mut cur_en = self.apply_disable(guard);
                for step in 0..min {
                    if step > 0 {
                        cur_en = self.make_sva_delay_reg(cur_en);
                    }
                    self.add_property(assert_kind, cond, cur_en, loc);
                }
                Ok(())
            }
            "goto" | "nonconsecutive" => {
                // Track active occurrence count until `min` hits of `cond` occur (checked as liveness if unbounded, or safety).
                let n = min.max(1) as u64;
                let cnt_w = (64 - n.leading_zeros()).max(2);
                let pend_name = format!("$sva_rep_pend_{}", self.past_counter);
                self.past_counter += 1;
                let pend_sig = self.module.add_signal(pend_name, 1, false, SignalKind::Reg);
                self.module.signal_mut(pend_sig).init = Some(ConstBits::zero(1));
                let pend_e = self.module.sig_expr(pend_sig);

                let cnt_name = format!("$sva_rep_cnt_{}", self.past_counter);
                self.past_counter += 1;
                let cnt_sig = self.module.add_signal(cnt_name, cnt_w, false, SignalKind::Reg);
                self.module.signal_mut(cnt_sig).init = Some(ConstBits::zero(cnt_w));
                let cnt_e = self.module.sig_expr(cnt_sig);

                let g_eff = self.apply_disable(guard);
                let active = self.module.logic_or(pend_e, g_eff);
                let one_c = self.module.c_u64(cnt_w, 1);
                let zero_c = self.module.c_u64(cnt_w, 0);
                let base_cnt = self.module.mux(g_eff, zero_c, cnt_e);
                let inc_cnt = self.module.intern(cnt_w, ExprKind::Add(base_cnt, one_c));
                let next_cnt_raw = self.module.mux(cond, inc_cnt, base_cnt);
                let target_c = self.module.c_u64(cnt_w, n);
                let reached = self.module.intern(1, ExprKind::Ule(target_c, next_cnt_raw));
                let not_reached = self.module.logic_not(reached);
                let next_pend_raw = self.module.logic_and(active, not_reached);
                let next_pend = self.apply_disable(next_pend_raw);
                let next_cnt = if let Some(dis) = self.cur_disable {
                    self.module.mux(dis, zero_c, next_cnt_raw)
                } else {
                    next_cnt_raw
                };

                self.aux_ffs.push(Ff {
                    signal: pend_sig,
                    next: next_pend,
                    init: Some(ConstBits::zero(1)),
                    clock: None,
                    async_reset: None,
                });
                self.aux_ffs.push(Ff {
                    signal: cnt_sig,
                    next: next_cnt,
                    init: Some(ConstBits::zero(cnt_w)),
                    clock: None,
                    async_reset: None,
                });

                let not_pend = self.module.logic_not(pend_e);
                let ok = self.module.logic_or(not_pend, reached);
                let one = self.module.c_bool(true);
                self.add_property("live", ok, one, loc);
                Ok(())
            }
            other => bail!("unsupported SVA repetition kind '{other}' at {loc}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_sva_until(
        &mut self,
        assert_kind: &str,
        p: ExprId,
        q: ExprId,
        guard: ExprId,
        strong: bool,
        inclusive: bool,
        loc: &str,
    ) {
        let g_eff = self.apply_disable(guard);
        let act_name = format!("$sva_until_{}", self.past_counter);
        self.past_counter += 1;
        let act_sig = self.module.add_signal(act_name, 1, false, SignalKind::Reg);
        self.module.signal_mut(act_sig).init = Some(ConstBits::zero(1));
        let act_e = self.module.sig_expr(act_sig);

        let now_active = self.module.logic_or(act_e, g_eff);
        let not_q = self.module.logic_not(q);
        let next_act_raw = self.module.logic_and(now_active, not_q);
        let next_act = self.apply_disable(next_act_raw);
        self.aux_ffs.push(Ff {
            signal: act_sig,
            next: next_act,
            init: Some(ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });

        let check_en = if inclusive {
            self.apply_disable(now_active)
        } else {
            let before_q = self.module.logic_and(now_active, not_q);
            self.apply_disable(before_q)
        };
        self.add_property(assert_kind, p, check_en, loc);

        if strong {
            let not_act = self.module.logic_not(now_active);
            let done = self.module.logic_or(not_act, q);
            let one = self.module.c_bool(true);
            self.add_property("live", done, one, loc);
        }
    }

    fn apply_disable(&mut self, guard: ExprId) -> ExprId {
        if let Some(dis) = self.cur_disable {
            let not_dis = self.module.logic_not(dis);
            self.module.logic_and(guard, not_dis)
        } else {
            guard
        }
    }

    fn make_sva_delay_reg(&mut self, trig: ExprId) -> ExprId {
        let gated_in = self.apply_disable(trig);
        let delayed = self.make_past_reg(gated_in, Some(ConstBits::zero(1)));
        self.apply_disable(delayed)
    }

    fn lower_assertion_bool(
        &mut self,
        prop: &AssertionExpr,
        env: &HashMap<SignalId, ExprId>,
    ) -> Result<ExprId> {
        match prop {
            AssertionExpr::Simple { expr, repetition } => {
                let c = self.lower_expr(expr, env)?;
                let cb = self.module.to_bool(c);
                if let Some(rep) = repetition {
                    self.lower_antecedent_repetition(cb, rep)
                } else {
                    Ok(cb)
                }
            }
            AssertionExpr::SeqWithMatch { expr, repetition } => {
                let cb = self.lower_assertion_bool(expr, env)?;
                if let Some(rep) = repetition {
                    self.lower_antecedent_repetition(cb, rep)
                } else {
                    Ok(cb)
                }
            }
            AssertionExpr::FirstMatch { expr } | AssertionExpr::StrongWeak { expr } => {
                self.lower_assertion_bool(expr, env)
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
            AssertionExpr::Binary { op, left, right } if op == "Iff" => {
                let a = self.lower_assertion_bool(left, env)?;
                let b = self.lower_assertion_bool(right, env)?;
                Ok(self.module.eq(a, b, false))
            }
            AssertionExpr::Binary { op, left, right } if op == "Throughout" => {
                let a = self.lower_assertion_bool(left, env)?;
                let b = self.lower_assertion_bool(right, env)?;
                Ok(self.module.logic_and(a, b))
            }
            AssertionExpr::SeqConcat { elements } => {
                // Sequence match indicator for antecedent `a ##1 b |-> c`
                let mut matched = self.module.c_bool(true);
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        for _ in 0..el.delay_min {
                            matched = self.make_sva_delay_reg(matched);
                        }
                    }
                    let c = self.lower_assertion_bool(&el.expr, env)?;
                    matched = self.module.logic_and(matched, c);
                }
                Ok(matched)
            }
            _ => bail!("nested temporal SVA expression not yet supported"),
        }
    }

    fn lower_antecedent_repetition(&mut self, cond: ExprId, rep: &SeqRepetition) -> Result<ExprId> {
        let min = rep.min as usize;
        match rep.kind.as_str() {
            "consecutive" => {
                if min == 0 {
                    return Ok(self.module.c_bool(true));
                }
                let mut streak = cond;
                for _ in 1..min {
                    let prev_streak = self.make_sva_delay_reg(streak);
                    streak = self.module.logic_and(prev_streak, cond);
                }
                Ok(streak)
            }
            "goto" | "nonconsecutive" => {
                let n = min.max(1) as u64;
                let cnt_w = (64 - n.leading_zeros()).max(2);
                let cnt_name = format!("$sva_ant_cnt_{}", self.past_counter);
                self.past_counter += 1;
                let cnt_sig = self.module.add_signal(cnt_name, cnt_w, false, SignalKind::Reg);
                self.module.signal_mut(cnt_sig).init = Some(ConstBits::zero(cnt_w));
                let cnt_e = self.module.sig_expr(cnt_sig);

                let one_c = self.module.c_u64(cnt_w, 1);
                let zero_c = self.module.c_u64(cnt_w, 0);
                let target_c = self.module.c_u64(cnt_w, n);
                let inc = self.module.intern(cnt_w, ExprKind::Add(cnt_e, one_c));
                let at_max = self.module.intern(1, ExprKind::Ule(target_c, cnt_e));
                let sat_inc = self.module.mux(at_max, cnt_e, inc);
                let next_raw = self.module.mux(cond, sat_inc, cnt_e);
                let next_cnt = if let Some(dis) = self.cur_disable {
                    self.module.mux(dis, zero_c, next_raw)
                } else {
                    next_raw
                };
                self.aux_ffs.push(Ff {
                    signal: cnt_sig,
                    next: next_cnt,
                    init: Some(ConstBits::zero(cnt_w)),
                    clock: None,
                    async_reset: None,
                });

                let cur_plus = self.module.mux(cond, sat_inc, cnt_e);
                let ge_n = self.module.intern(1, ExprKind::Ule(target_c, cur_plus));
                if rep.kind == "goto" {
                    Ok(self.module.logic_and(ge_n, cond))
                } else {
                    Ok(ge_n)
                }
            }
            other => bail!("unsupported SVA repetition kind '{other}'"),
        }
    }

    fn add_property(&mut self, kind_str: &str, cond: ExprId, en: ExprId, loc: &str) {
        let kind = match kind_str {
            "assume" => PropertyKind::Assume,
            "cover" => PropertyKind::Cover,
            "live" => PropertyKind::Live,
            "fair" => PropertyKind::Fair,
            _ => PropertyKind::Assert,
        };
        let idx = self.prop_counter;
        self.prop_counter += 1;
        let base = if let Some(lbl) = &self.cur_label {
            lbl.clone()
        } else {
            format!("{kind_str}_{idx}")
        };
        let name = if loc.is_empty() {
            base
        } else {
            format!("{base}@{loc}")
        };
        self.add_named_property(kind, name, cond, en, loc);
    }

    fn add_named_property(
        &mut self,
        kind: PropertyKind,
        name: String,
        cond: ExprId,
        en: ExprId,
        loc: &str,
    ) {
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
        let mut keys: Vec<SignalId> = then_map.keys().chain(else_map.keys()).copied().collect();
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

    #[allow(clippy::too_many_arguments)]
    fn apply_write(
        &mut self,
        lhs: &Expr,
        rhs: ExprId,
        guard: ExprId,
        target: &mut HashMap<SignalId, ExprId>,
        mem_writes: &mut Vec<MemWrite>,
        is_nba: bool,
        clock: Option<ClockSpec>,
    ) -> Result<()> {
        match lhs {
            Expr::Ref { name, width, .. } => {
                if self.local_exprs.contains_key(name) {
                    let val = self.module.resize(rhs, *width, false);
                    self.local_exprs.insert(name.clone(), val);
                    return Ok(());
                }
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
                    self.apply_write(op, slice, guard, target, mem_writes, is_nba, clock)?;
                }
                Ok(())
            }
            Expr::FieldSelect {
                value,
                bit_offset,
                width,
                ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(sig) = self.module.find_signal(name) {
                        let base = target
                            .get(&sig)
                            .copied()
                            .unwrap_or_else(|| self.module.sig_expr(sig));
                        let lo = *bit_offset as u32;
                        let hi = lo + *width - 1;
                        let updated = self.module.update_slice(base, hi, lo, rhs);
                        target.insert(sig, updated);
                        return Ok(());
                    }
                }
                bail!("unsupported packed struct field assignment LHS")
            }
            Expr::BitSelect {
                width,
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
                        let elem_w = (*width).max(1);
                        if let Some(idx) =
                            self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64())
                        {
                            let elem_idx =
                                self.normalize_bit_index(idx as i64, *range_left, *range_right);
                            let lo = elem_idx * elem_w;
                            let hi = lo + elem_w - 1;
                            let updated = self.module.update_slice(base, hi, lo, rhs);
                            target.insert(sig, updated);
                            return Ok(());
                        } else {
                            let w = self.module.signal(sig).width;
                            let num_elems = (w / elem_w).max(1);
                            let sel_e = self.lower_expr(selector, target)?;
                            if self.inst.oob_bitselect_checks {
                                self.emit_bitselect_oob_check(
                                    "write",
                                    sel_e,
                                    selector.is_signed(),
                                    num_elems,
                                    *range_left,
                                    *range_right,
                                    guard,
                                );
                            }
                            let sel_w = self.module.resize(sel_e, w, false);
                            let bit_shift = if elem_w == 1 {
                                sel_w
                            } else {
                                let ew_c = self.module.c_u64(w, elem_w as u64);
                                self.module.intern(w, ExprKind::Mul(sel_w, ew_c))
                            };
                            let elem_mask = self.module.c_bits(
                                ConstBits::new(elem_w, "1".repeat(elem_w as usize)).resize_zext(w),
                            );
                            let mask = self.module.intern(w, ExprKind::Shl(elem_mask, bit_shift));
                            let not_mask = self.module.not(mask);
                            let cleared = self.module.intern(w, ExprKind::And(base, not_mask));
                            let rhs_sized = self.module.resize(rhs, elem_w, false);
                            let rhs_zext = self.module.resize(rhs_sized, w, false);
                            let shifted = self.module.intern(w, ExprKind::Shl(rhs_zext, bit_shift));
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
                        let depth = mem.depth;
                        let mem_name = mem.name.clone();
                        let addr_raw = self.lower_expr(selector, target)?;
                        if self.inst.oob_memory_checks {
                            let rw = self.module.width(addr_raw).max(aw + 1);
                            let addr_ext = self.module.resize(addr_raw, rw, false);
                            let depth_c = self.module.c_u64(rw, depth as u64);
                            let in_bounds = self.module.intern(1, ExprKind::Ult(addr_ext, depth_c));
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_oob_mem_write_{mem_name}_{idx}"),
                                in_bounds,
                                guard,
                                "",
                            );
                        }
                        let addr = self.module.resize(addr_raw, aw, false);
                        let data = self.module.resize(rhs, dw, false);
                        mem_writes.push(MemWrite {
                            mem: mem_id,
                            addr,
                            data,
                            enable: guard,
                            clock,
                        });
                        return Ok(());
                    }
                }
                bail!("unknown memory in assignment LHS")
            }
            _ => bail!("unsupported assignment LHS: {lhs:?}"),
        }
    }

    fn normalize_bit_index(
        &self,
        idx: i64,
        range_left: Option<i64>,
        range_right: Option<i64>,
    ) -> u32 {
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
            clock: None,
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
            clock: None,
            async_reset: None,
        });
        self.module.sig_expr(sig)
    }

    fn lower_expr(&mut self, expr: &Expr, env: &HashMap<SignalId, ExprId>) -> Result<ExprId> {
        match expr {
            Expr::Const { width, bits, .. } => {
                Ok(self.module.c_bits(ConstBits::new(*width, bits.clone())))
            }
            Expr::Ref { name, width, .. } => {
                if let Some(&val) = self.const_locals.get(name) {
                    return Ok(self.module.c_bits(ConstBits::from_i64(*width, val)));
                }
                if let Some(&val) = self.local_exprs.get(name) {
                    return Ok(self.module.resize(val, *width, false));
                }
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
                let src_w = self.module.width(v);
                if self.inst.truncation_checks && src_w > *width {
                    let drop_w = src_w - *width;
                    let upper = self.module.slice(v, src_w - 1, *width);
                    let zeros = self.module.c_u64(drop_w, 0);
                    let no_loss = if op_signed {
                        let ones = self.module.c_bits(ConstBits::new(drop_w, "1".repeat(drop_w as usize)));
                        let sign_bit = self.module.slice(v, *width - 1, *width - 1);
                        let upper_zero = self.module.intern(1, ExprKind::Eq(upper, zeros));
                        let upper_ones = self.module.intern(1, ExprKind::Eq(upper, ones));
                        self.module.mux(sign_bit, upper_ones, upper_zero)
                    } else {
                        self.module.intern(1, ExprKind::Eq(upper, zeros))
                    };
                    let trivially_safe = match &self.module.expr(v).kind {
                        ExprKind::Const(c) => {
                            let hi_bits = &c.bits[..drop_w as usize];
                            if op_signed {
                                let sb = c.bits.as_bytes()[drop_w as usize] as char;
                                hi_bits.chars().all(|ch| ch == sb)
                            } else {
                                hi_bits.chars().all(|ch| ch == '0')
                            }
                        }
                        _ => false,
                    };
                    if !trivially_safe {
                        let idx = self.prop_counter;
                        self.prop_counter += 1;
                        let guard = self.cur_guard;
                        self.add_named_property(
                            PropertyKind::Assert,
                            format!("autocheck_width_truncation_{idx}"),
                            no_loss,
                            guard,
                            "",
                        );
                    }
                }
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
                        if self.inst.div_zero_checks && matches!(op.as_str(), "Divide" | "Mod") {
                            let zero = self.module.c_u64(*width, 0);
                            let nonzero = self.module.intern(1, ExprKind::Ne(r, zero));
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            let guard = self.cur_guard;
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_div_by_zero_{idx}"),
                                nonzero,
                                guard,
                                "",
                            );
                        }
                        if self.inst.overflow_checks && matches!(op.as_str(), "Add" | "Subtract") {
                            if !*signed {
                                let ew = *width + 1;
                                let le = self.module.resize(l, ew, false);
                                let re = self.module.resize(r, ew, false);
                                let ext_op = if op == "Add" {
                                    self.module.intern(ew, ExprKind::Add(le, re))
                                } else {
                                    self.module.intern(ew, ExprKind::Sub(le, re))
                                };
                                let carry = self.module.slice(ext_op, *width, *width);
                                let no_ovf = self.module.logic_not(carry);
                                let idx = self.prop_counter;
                                self.prop_counter += 1;
                                let guard = self.cur_guard;
                                self.add_named_property(
                                    PropertyKind::Assert,
                                    format!("autocheck_{}_overflow_{idx}", op.to_ascii_lowercase()),
                                    no_ovf,
                                    guard,
                                    "",
                                );
                            } else if *width >= 2 {
                                let res_s = if op == "Add" {
                                    self.module.intern(*width, ExprKind::Add(l, r))
                                } else {
                                    self.module.intern(*width, ExprKind::Sub(l, r))
                                };
                                let msb = *width - 1;
                                let l_msb = self.module.slice(l, msb, msb);
                                let r_msb = self.module.slice(r, msb, msb);
                                let res_msb = self.module.slice(res_s, msb, msb);
                                let sign_cond = if op == "Add" {
                                    self.module.intern(1, ExprKind::Eq(l_msb, r_msb))
                                } else {
                                    self.module.intern(1, ExprKind::Ne(l_msb, r_msb))
                                };
                                let flipped = self.module.intern(1, ExprKind::Ne(res_msb, l_msb));
                                let ovf = self.module.logic_and(sign_cond, flipped);
                                let no_ovf = self.module.logic_not(ovf);
                                let idx = self.prop_counter;
                                self.prop_counter += 1;
                                let guard = self.cur_guard;
                                self.add_named_property(
                                    PropertyKind::Assert,
                                    format!("autocheck_s{}_overflow_{idx}", op.to_ascii_lowercase()),
                                    no_ovf,
                                    guard,
                                    "",
                                );
                            }
                        }
                        if self.inst.overflow_checks && op == "Multiply" && *width >= 2 && *width <= 64 {
                            let ew = *width * 2;
                            let le = self.module.resize(l, ew, *signed);
                            let re = self.module.resize(r, ew, *signed);
                            let prod = self.module.intern(ew, ExprKind::Mul(le, re));
                            let upper = self.module.slice(prod, ew - 1, *width);
                            let zeros = self.module.c_u64(*width, 0);
                            let no_ovf = if *signed {
                                let ones = self
                                    .module
                                    .c_bits(ConstBits::new(*width, "1".repeat(*width as usize)));
                                let sign_bit = self.module.slice(prod, *width - 1, *width - 1);
                                let u_zero = self.module.intern(1, ExprKind::Eq(upper, zeros));
                                let u_ones = self.module.intern(1, ExprKind::Eq(upper, ones));
                                self.module.mux(sign_bit, u_ones, u_zero)
                            } else {
                                self.module.intern(1, ExprKind::Eq(upper, zeros))
                            };
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            let guard = self.cur_guard;
                            let tag = if *signed { "smul" } else { "mul" };
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_{tag}_overflow_{idx}"),
                                no_ovf,
                                guard,
                                "",
                            );
                        }
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
                    "LogicalShiftLeft" | "ArithmeticShiftLeft" | "LogicalShiftRight"
                    | "ArithmeticShiftRight" => {
                        if self.inst.shift_overflow_checks {
                            let trivial_const = match &self.module.expr(r).kind {
                                ExprKind::Const(c) => c.to_u64().is_some_and(|v| v < (*width as u64)),
                                _ => false,
                            };
                            if !trivial_const {
                                let rw = self.module.width(r);
                                let needed = ((*width as u64).max(1).ilog2() + 2).max(2);
                                let cmp_w = rw.max(needed);
                                let re = self.module.resize(r, cmp_w, false);
                                let limit = self.module.c_u64(cmp_w, *width as u64);
                                let ok = self.module.intern(1, ExprKind::Ult(re, limit));
                                let idx = self.prop_counter;
                                self.prop_counter += 1;
                                let guard = self.cur_guard;
                                self.add_named_property(
                                    PropertyKind::Assert,
                                    format!("autocheck_shift_overflow_{idx}"),
                                    ok,
                                    guard,
                                    "",
                                );
                            }
                        }
                        let l = self.module.resize(l, *width, l_signed);
                        let r = self.module.resize(r, *width, false);
                        let kind = match op.as_str() {
                            "LogicalShiftLeft" | "ArithmeticShiftLeft" => ExprKind::Shl(l, r),
                            "LogicalShiftRight" => ExprKind::Lshr(l, r),
                            "ArithmeticShiftRight" => {
                                if *signed || l_signed {
                                    ExprKind::Ashr(l, r)
                                } else {
                                    ExprKind::Lshr(l, r)
                                }
                            }
                            _ => unreachable!(),
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
                let c_b = self.module.to_bool(c);
                let not_c = self.module.logic_not(c_b);
                let saved_g = self.cur_guard;
                self.cur_guard = self.module.logic_and(saved_g, c_b);
                let t = self.lower_expr(then, env)?;
                self.cur_guard = self.module.logic_and(saved_g, not_c);
                let e = self.lower_expr(else_expr, env)?;
                self.cur_guard = saved_g;
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
            Expr::FieldSelect {
                width,
                bit_offset,
                value,
                ..
            } => {
                let base = self.lower_expr(value, env)?;
                let lo = *bit_offset as u32;
                let hi = lo + *width - 1;
                Ok(self.module.slice(base, hi, lo))
            }
            Expr::BitSelect {
                width,
                value,
                selector,
                range_left,
                range_right,
                ..
            } => {
                let base = self.lower_expr(value, env)?;
                let elem_w = (*width).max(1);
                if let Some(idx) =
                    self.try_eval_const(selector, 32, true).and_then(|c| c.to_u64())
                {
                    let elem_idx =
                        self.normalize_bit_index(idx as i64, *range_left, *range_right);
                    let lo = elem_idx * elem_w;
                    let hi = lo + elem_w - 1;
                    Ok(self.module.slice(base, hi, lo))
                } else {
                    let w = self.module.width(base);
                    let num_elems = (w / elem_w).max(1);
                    let sel = self.lower_expr(selector, env)?;
                    if self.inst.oob_bitselect_checks {
                        let guard = self.cur_guard;
                        self.emit_bitselect_oob_check(
                            "read",
                            sel,
                            selector.is_signed(),
                            num_elems,
                            *range_left,
                            *range_right,
                            guard,
                        );
                    }
                    let sel_w = self.module.resize(sel, w, false);
                    let bit_shift = if elem_w == 1 {
                        sel_w
                    } else {
                        let ew_c = self.module.c_u64(w, elem_w as u64);
                        self.module.intern(w, ExprKind::Mul(sel_w, ew_c))
                    };
                    let shifted = self.module.intern(w, ExprKind::Lshr(base, bit_shift));
                    Ok(self.module.slice(shifted, elem_w - 1, 0))
                }
            }
            Expr::Inside { left, ranges, .. } => {
                let lv = self.lower_expr(left, env)?;
                let l_signed = left.is_signed();
                let mut acc = self.module.c_bool(false);
                for item in ranges {
                    let m = self.lower_case_match("inside", lv, l_signed, item, env)?;
                    acc = self.module.logic_or(acc, m);
                }
                Ok(acc)
            }
            Expr::ValueRange { left, .. } => self.lower_expr(left, env),
            Expr::Streaming {
                width,
                slice_size,
                operands,
                ..
            } => {
                let mut parts = Vec::with_capacity(operands.len());
                for op in operands {
                    parts.push(self.lower_expr(op, env)?);
                }
                let packed = if parts.is_empty() {
                    self.module.c_u64((*width).max(1), 0)
                } else {
                    self.module.concat(parts)
                };
                let total_w = self.module.width(packed);
                let res = if *slice_size == 0 {
                    // `{>>{...}}`: left-to-right bitstream packing.
                    packed
                } else {
                    // `{<<slice_size{...}}`: reverse chunks of `slice_size` bits.
                    let chunk = (*slice_size as u32).max(1);
                    let mut chunks = Vec::new();
                    let mut hi = total_w;
                    while hi > 0 {
                        let take = chunk.min(hi);
                        let lo = hi - take;
                        chunks.push(self.module.slice(packed, hi - 1, lo));
                        hi = lo;
                    }
                    chunks.reverse();
                    self.module.concat(chunks)
                };
                Ok(self.module.resize(res, (*width).max(1), false))
            }
            Expr::MemSelect {
                value, selector, ..
            } => {
                if let Expr::Ref { name, .. } = value.as_ref() {
                    if let Some(&mem_id) = self.module.mem_by_name.get(name) {
                        let mem = self.module.memory(mem_id);
                        let aw = mem.addr_width;
                        let dw = mem.data_width;
                        let depth = mem.depth;
                        let mem_name = mem.name.clone();
                        let addr_raw = self.lower_expr(selector, env)?;
                        if self.inst.oob_memory_checks {
                            let rw = self.module.width(addr_raw).max(aw + 1);
                            let addr_ext = self.module.resize(addr_raw, rw, false);
                            let depth_c = self.module.c_u64(rw, depth as u64);
                            let in_bounds = self.module.intern(1, ExprKind::Ult(addr_ext, depth_c));
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            let guard = self.cur_guard;
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_oob_mem_read_{mem_name}_{idx}"),
                                in_bounds,
                                guard,
                                "",
                            );
                        }
                        let addr = self.module.resize(addr_raw, aw, false);
                        return Ok(self
                            .module
                            .intern(dw, ExprKind::MemRead { mem: mem_id, addr }));
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
                        let shift_raw = self.lower_expr(left.as_ref().unwrap(), env)?;
                        if self.inst.oob_bitselect_checks {
                            let sw = self.module.width(shift_raw);
                            let cmp_w = sw.max(((w as u64).max(1).ilog2() + 3).max(4));
                            let se = self.module.resize(shift_raw, cmp_w, false);
                            let slice_w = self.module.c_u64(cmp_w, *width as u64);
                            let end = self.module.intern(cmp_w, ExprKind::Add(se, slice_w));
                            let limit = self.module.c_u64(cmp_w, w as u64);
                            let in_bounds = self.module.intern(1, ExprKind::Ule(end, limit));
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            let guard = self.cur_guard;
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_oob_bitselect_part_{idx}"),
                                in_bounds,
                                guard,
                                "",
                            );
                        }
                        let shift = self.module.resize(shift_raw, w, false);
                        let shifted = self.module.intern(w, ExprKind::Lshr(base, shift));
                        Ok(self.module.slice(shifted, *width - 1, 0))
                    }
                    ("indexed_down", None, _) => {
                        let w = self.module.width(base);
                        let hi_raw = self.lower_expr(left.as_ref().unwrap(), env)?;
                        if self.inst.oob_bitselect_checks {
                            let sw = self.module.width(hi_raw);
                            let cmp_w = sw.max(((w as u64).max(1).ilog2() + 3).max(4));
                            let he = self.module.resize(hi_raw, cmp_w, false);
                            let min_hi = self.module.c_u64(cmp_w, (*width - 1) as u64);
                            let max_hi = self.module.c_u64(cmp_w, (w - 1) as u64);
                            let ge_min = self.module.intern(1, ExprKind::Ule(min_hi, he));
                            let le_max = self.module.intern(1, ExprKind::Ule(he, max_hi));
                            let in_bounds = self.module.logic_and(ge_min, le_max);
                            let idx = self.prop_counter;
                            self.prop_counter += 1;
                            let guard = self.cur_guard;
                            self.add_named_property(
                                PropertyKind::Assert,
                                format!("autocheck_oob_bitselect_part_{idx}"),
                                in_bounds,
                                guard,
                                "",
                            );
                        }
                        let hi_w = self.module.resize(hi_raw, w, false);
                        let sub_amt = self.module.c_u64(w, (*width - 1) as u64);
                        let shift = self.module.intern(w, ExprKind::Sub(hi_w, sub_amt));
                        let shifted = self.module.intern(w, ExprKind::Lshr(base, shift));
                        Ok(self.module.slice(shifted, *width - 1, 0))
                    }
                    _ => bail!("unsupported dynamic range select"),
                }
            }
            Expr::FuncCall {
                name,
                width,
                signed,
                params,
                args,
                body,
            } => {
                // Inline pure combinational SystemVerilog function call.
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(self.lower_expr(a, env)?);
                }
                let saved_locals = std::mem::take(&mut self.local_exprs);
                for (p, v) in params.iter().zip(arg_vals) {
                    self.local_exprs.insert(p.clone(), v);
                }
                let zero = self.module.c_u64(*width, 0);
                self.local_exprs.insert(name.clone(), zero);

                let mut fn_env = env.clone();
                let mut fn_nba = HashMap::new();
                let mut fn_mw = Vec::new();
                let en = self.module.c_bool(true);
                self.lower_stmt(body, en, &mut fn_env, &mut fn_nba, &mut fn_mw, false, None)?;

                let ret = self
                    .local_exprs
                    .get("$return")
                    .or_else(|| self.local_exprs.get(name))
                    .copied()
                    .unwrap_or(zero);
                self.local_exprs = saved_locals;
                Ok(self.module.resize(ret, *width, *signed))
            }
            Expr::Call {
                name,
                width,
                signed,
                args,
            } => self.lower_system_call(name, *width, *signed, args, env),
            Expr::AssignExpr { rhs, .. } => self.lower_expr(rhs, env),
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
            "$countones" => {
                let v = self.lower_expr(&args[0], env)?;
                let vw = self.module.width(v);
                let mut acc = self.module.c_u64(width, 0);
                for bit in 0..vw {
                    let b = self.module.slice(v, bit, bit);
                    let bz = self.module.resize(b, width, false);
                    acc = self.module.intern(width, ExprKind::Add(acc, bz));
                }
                Ok(acc)
            }
            other => bail!("unsupported system function call '{other}'"),
        }
    }

    fn eval_loop_step(&self, var: &str, step: &Expr) -> Result<i64> {
        let cur = *self
            .const_locals
            .get(var)
            .ok_or_else(|| anyhow!("missing loop var '{var}'"))?;
        match step {
            Expr::AssignExpr { rhs, .. } => self
                .try_eval_i64(rhs)
                .ok_or_else(|| anyhow!("non-constant for-loop step expression")),
            Expr::Unary { op, .. } if op == "Postincrement" || op == "Preincrement" => Ok(cur + 1),
            Expr::Unary { op, .. } if op == "Postdecrement" || op == "Predecrement" => Ok(cur - 1),
            _ => self
                .try_eval_i64(step)
                .ok_or_else(|| anyhow!("unsupported for-loop step: {step:?}")),
        }
    }

    fn try_eval_i64(&self, expr: &Expr) -> Option<i64> {
        match expr {
            Expr::Const { width, signed, bits, .. } => {
                let c = ConstBits::new(*width, bits.clone());
                let raw = c.to_u64()?;
                if *signed && *width < 64 && ((raw >> (*width - 1)) & 1) != 0 {
                    let ext = !((1u64 << *width) - 1);
                    Some((raw | ext) as i64)
                } else {
                    Some(raw as i64)
                }
            }
            Expr::Ref { name, .. } => self.const_locals.get(name).copied(),
            Expr::Conv { operand, .. } => self.try_eval_i64(operand),
            Expr::Unary { op, operand, .. } => {
                let v = self.try_eval_i64(operand)?;
                match op.as_str() {
                    "Plus" => Some(v),
                    "Minus" => Some(-v),
                    "LogicalNot" => Some(i64::from(v == 0)),
                    _ => None,
                }
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.try_eval_i64(left)?;
                let r = self.try_eval_i64(right)?;
                match op.as_str() {
                    "Add" => Some(l.wrapping_add(r)),
                    "Subtract" => Some(l.wrapping_sub(r)),
                    "Multiply" => Some(l.wrapping_mul(r)),
                    "Divide" if r != 0 => Some(l / r),
                    "LessThan" => Some(i64::from(l < r)),
                    "LessThanEqual" => Some(i64::from(l <= r)),
                    "GreaterThan" => Some(i64::from(l > r)),
                    "GreaterThanEqual" => Some(i64::from(l >= r)),
                    "Equality" => Some(i64::from(l == r)),
                    "Inequality" => Some(i64::from(l != r)),
                    "LogicalAnd" => Some(i64::from(l != 0 && r != 0)),
                    "LogicalOr" => Some(i64::from(l != 0 || r != 0)),
                    _ => None,
                }
            }
            _ => None,
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
            Expr::Ref { name, .. } => self
                .const_locals
                .get(name)
                .map(|&v| ConstBits::from_i64(target_width, v)),
            Expr::Conv { operand, .. } => {
                self.try_eval_const(operand, target_width, operand.is_signed())
            }
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
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.try_eval_const(left, target_width, signed)?.to_u128()?;
                let r = self
                    .try_eval_const(right, target_width, signed)?
                    .to_u128()?;
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
    fn emit_bitselect_oob_check(
        &mut self,
        dir: &str,
        sel: ExprId,
        sel_signed: bool,
        vec_width: u32,
        range_left: Option<i64>,
        range_right: Option<i64>,
        guard: ExprId,
    ) {
        let (lo_i, hi_i) = match (range_left, range_right) {
            (Some(l), Some(r)) => (l.min(r), l.max(r)),
            _ => (0i64, vec_width as i64 - 1),
        };
        let sw = self.module.width(sel);
        // Statically in-bounds unsigned selector on `[vec_width-1:0]` needs no assertion.
        if !sel_signed && lo_i == 0 && hi_i == (vec_width as i64 - 1) && sw < 64 && (1u128 << sw) <= vec_width as u128 {
            return;
        }
        let in_bounds = if !sel_signed && lo_i >= 0 {
            let max_u = hi_i as u64;
            let needed = (max_u.max(1).ilog2() + 2).max(2);
            let cmp_w = sw.max(needed);
            let se = self.module.resize(sel, cmp_w, false);
            let hi_c = self.module.c_u64(cmp_w, max_u);
            let le_hi = self.module.intern(1, ExprKind::Ule(se, hi_c));
            if lo_i == 0 {
                le_hi
            } else {
                let lo_c = self.module.c_u64(cmp_w, lo_i as u64);
                let ge_lo = self.module.intern(1, ExprKind::Ule(lo_c, se));
                self.module.logic_and(ge_lo, le_hi)
            }
        } else {
            let cmp_w = sw.max(32) + 1;
            let se = self.module.resize(sel, cmp_w, sel_signed);
            let lo_c = self.module.c_bits(ConstBits::from_i64(cmp_w, lo_i));
            let hi_c = self.module.c_bits(ConstBits::from_i64(cmp_w, hi_i));
            let ge_lo = self.module.intern(1, ExprKind::Sle(lo_c, se));
            let le_hi = self.module.intern(1, ExprKind::Sle(se, hi_c));
            self.module.logic_and(ge_lo, le_hi)
        };
        let idx = self.prop_counter;
        self.prop_counter += 1;
        self.add_named_property(
            PropertyKind::Assert,
            format!("autocheck_oob_bitselect_{dir}_{idx}"),
            in_bounds,
            guard,
            "",
        );
    }
}

/// Simplify nested `Slice(Concat(...), hi, lo)` nodes created by sequential bit-slice updates
/// (e.g. `y[0] = a; y[1] = b;`) so fully assigned vectors have no residual self-references.
fn collapse_slice_concat(module: &mut Module, expr: ExprId) -> ExprId {
    let mut cache = HashMap::new();
    collapse_slice_concat_inner(module, expr, &mut cache)
}

fn collapse_slice_concat_inner(
    module: &mut Module,
    expr: ExprId,
    cache: &mut HashMap<ExprId, ExprId>,
) -> ExprId {
    if let Some(&r) = cache.get(&expr) {
        return r;
    }
    let node = module.expr(expr).clone();
    let res = match node.kind {
        ExprKind::Slice { expr: inner, hi, lo } => {
            let inner = collapse_slice_concat_inner(module, inner, cache);
            if let ExprKind::Concat(parts) = module.expr(inner).kind.clone() {
                let total_w = module.width(inner);
                let mut cur_hi = total_w;
                let mut out_parts = Vec::new();
                for p in parts {
                    let pw = module.width(p);
                    let p_lo = cur_hi - pw;
                    let p_hi = cur_hi - 1;
                    cur_hi = p_lo;
                    if hi >= p_lo && lo <= p_hi {
                        let sub_hi = hi.min(p_hi) - p_lo;
                        let sub_lo = lo.max(p_lo) - p_lo;
                        let sliced = if sub_hi == pw - 1 && sub_lo == 0 {
                            p
                        } else {
                            module.slice(p, sub_hi, sub_lo)
                        };
                        out_parts.push(sliced);
                    }
                }
                if !out_parts.is_empty() {
                    module.concat(out_parts)
                } else {
                    module.slice(inner, hi, lo)
                }
            } else {
                module.slice(inner, hi, lo)
            }
        }
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let c = collapse_slice_concat_inner(module, cond, cache);
            let t = collapse_slice_concat_inner(module, then_val, cache);
            let e = collapse_slice_concat_inner(module, else_val, cache);
            module.mux(c, t, e)
        }
        ExprKind::Concat(parts) => {
            let mapped: Vec<ExprId> = parts
                .iter()
                .map(|&p| collapse_slice_concat_inner(module, p, cache))
                .collect();
            module.concat(mapped)
        }
        _ => expr,
    };
    cache.insert(expr, res);
    res
}

/// Extract the exact combinational path predicate under which `expr` retains `Signal(sig)`
/// (indicating an unassigned path / inferred latch inside `always_comb`). Returns `None` if
/// every path assigns `sig` (or if `sig` appears only inside a genuine combinational feedback loop).
fn unassigned_path_cond(module: &mut Module, expr: ExprId, sig: SignalId) -> Option<ExprId> {
    match module.expr(expr).kind.clone() {
        ExprKind::Signal(s) if s == sig => Some(module.c_bool(true)),
        ExprKind::Slice { expr: inner, .. } => {
            if matches!(module.expr(inner).kind, ExprKind::Signal(s) if s == sig) {
                Some(module.c_bool(true))
            } else {
                None
            }
        }
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let t = unassigned_path_cond(module, then_val, sig);
            let e = unassigned_path_cond(module, else_val, sig);
            match (t, e) {
                (None, None) => None,
                (Some(tc), None) => Some(module.logic_and(cond, tc)),
                (None, Some(ec)) => {
                    let nc = module.logic_not(cond);
                    Some(module.logic_and(nc, ec))
                }
                (Some(tc), Some(ec)) => {
                    let a = module.logic_and(cond, tc);
                    let nc = module.logic_not(cond);
                    let b = module.logic_and(nc, ec);
                    Some(module.logic_or(a, b))
                }
            }
        }
        ExprKind::Concat(parts) => {
            let mut acc: Option<ExprId> = None;
            for p in parts {
                if let Some(pc) = unassigned_path_cond(module, p, sig) {
                    acc = Some(match acc {
                        None => pc,
                        Some(prev) => module.logic_or(prev, pc),
                    });
                }
            }
            acc
        }
        _ => None,
    }
}

/// Replace all leaf occurrences of `Signal(target)` inside `expr` with `replacement`.
fn replace_sig_in_expr(
    module: &mut Module,
    expr: ExprId,
    target: SignalId,
    replacement: ExprId,
    cache: &mut HashMap<ExprId, ExprId>,
) -> ExprId {
    if let Some(&r) = cache.get(&expr) {
        return r;
    }
    let node = module.expr(expr).clone();
    let res = match node.kind {
        ExprKind::Signal(s) if s == target => replacement,
        ExprKind::Const(_) | ExprKind::Signal(_) => expr,
        ExprKind::Not(a) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.intern(node.width, ExprKind::Not(a))
        }
        ExprKind::Neg(a) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.intern(node.width, ExprKind::Neg(a))
        }
        ExprKind::RedAnd(a) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.intern(1, ExprKind::RedAnd(a))
        }
        ExprKind::RedOr(a) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.intern(1, ExprKind::RedOr(a))
        }
        ExprKind::RedXor(a) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.intern(1, ExprKind::RedXor(a))
        }
        ExprKind::Slice { expr: a, hi, lo } => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.slice(a, hi, lo)
        }
        ExprKind::Zext { expr: a, width } => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.resize(a, width, false)
        }
        ExprKind::Sext { expr: a, width } => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            module.resize(a, width, true)
        }
        ExprKind::MemRead { mem, addr } => {
            let addr = replace_sig_in_expr(module, addr, target, replacement, cache);
            module.intern(node.width, ExprKind::MemRead { mem, addr })
        }
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => {
            let c = replace_sig_in_expr(module, cond, target, replacement, cache);
            let t = replace_sig_in_expr(module, then_val, target, replacement, cache);
            let e = replace_sig_in_expr(module, else_val, target, replacement, cache);
            module.mux(c, t, e)
        }
        ExprKind::Concat(parts) => {
            let mapped: Vec<ExprId> = parts
                .iter()
                .map(|&p| replace_sig_in_expr(module, p, target, replacement, cache))
                .collect();
            module.concat(mapped)
        }
        ExprKind::Add(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Add(a, b))
        }
        ExprKind::Sub(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Sub(a, b))
        }
        ExprKind::Mul(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Mul(a, b))
        }
        ExprKind::UDiv(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::UDiv(a, b))
        }
        ExprKind::SDiv(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::SDiv(a, b))
        }
        ExprKind::UMod(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::UMod(a, b))
        }
        ExprKind::SMod(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::SMod(a, b))
        }
        ExprKind::And(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::And(a, b))
        }
        ExprKind::Or(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Or(a, b))
        }
        ExprKind::Xor(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Xor(a, b))
        }
        ExprKind::Shl(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Shl(a, b))
        }
        ExprKind::Lshr(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Lshr(a, b))
        }
        ExprKind::Ashr(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(node.width, ExprKind::Ashr(a, b))
        }
        ExprKind::Eq(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Eq(a, b))
        }
        ExprKind::Ne(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Ne(a, b))
        }
        ExprKind::Ult(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Ult(a, b))
        }
        ExprKind::Ule(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Ule(a, b))
        }
        ExprKind::Slt(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Slt(a, b))
        }
        ExprKind::Sle(a, b) => {
            let a = replace_sig_in_expr(module, a, target, replacement, cache);
            let b = replace_sig_in_expr(module, b, target, replacement, cache);
            module.intern(1, ExprKind::Sle(a, b))
        }
    };
    cache.insert(expr, res);
    res
}

fn unwrap_single_stmt(stmt: &Stmt) -> Option<&Stmt> {
    match stmt {
        Stmt::Block { items, .. } if items.len() == 1 => unwrap_single_stmt(&items[0]),
        other => Some(other),
    }
}

fn extract_wildcard_const(expr: &Expr, mask_x: bool) -> Option<(ConstBits, ConstBits)> {
    match expr {
        Expr::Const {
            width,
            bits,
            mask_z,
            mask_xz,
            ..
        } => {
            let mask_str = if mask_x {
                mask_xz.as_ref()
            } else {
                mask_z.as_ref()
            }?;
            Some((
                ConstBits::new(*width, bits.clone()),
                ConstBits::new(*width, mask_str.clone()),
            ))
        }
        Expr::Conv {
            width, operand, ..
        } => {
            let (b, m) = extract_wildcard_const(operand, mask_x)?;
            Some((b.resize_zext(*width), m.resize_zext(*width)))
        }
        _ => None,
    }
}

pub fn lower_to_ir(design: &ElaboratedDesign) -> Result<Module> {
    Lowerer::new(&design.module, InstrumentOptions::default()).lower_design(design)
}

pub fn lower_to_ir_with_options(
    design: &ElaboratedDesign,
    inst: InstrumentOptions,
) -> Result<Module> {
    Lowerer::new(&design.module, inst).lower_design(design)
}
