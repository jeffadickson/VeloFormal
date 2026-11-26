use anyhow::Result;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{
    collect_signals, ConstBits, ExprId, ExprKind, Module, Property, PropertyKind, SignalId,
    SignalKind,
};
use vf_slang::{compile_to_ir_with_options, CompileOptions, InstrumentOptions};
use vf_smt::solver::TraceFrame;

/// All supported AutoCheck rule categories (usable with `--enable` / `--disable` and Questa Tcl).
pub const ALL_AUTOCHECK_CATEGORIES: &[&str] = &[
    "oob_memory",
    "oob_bitselect",
    "div_by_zero",
    "shift_overflow",
    "unique_priority",
    "inferred_latch",
    "overflow",
    "width_truncation",
    "comb_loop",
    "multi_driven",
    "clock_as_data",
    "undriven",
    "unread_signal",
    "const_reg",
    "no_reset_reg",
    "fsm_deadlock",
    "fsm_livelock",
    "fsm_unreachable",
];

#[derive(Debug, Clone)]
pub struct AutoCheckConfig {
    pub depth: usize,
    pub solver: String,
    /// Check unsigned & signed add/sub/mul arithmetic overflow/underflow.
    pub check_overflow: bool,
    /// Check formal implicit narrowing width truncation (silent data loss).
    pub check_truncation: bool,
    /// Check whether any initialized user FF is provably stuck constant forever (`reg == init`).
    pub check_const_regs: bool,
    /// Check formally whether any uninitialized FF is NOT cleared when the detected reset input is active.
    pub check_no_reset: bool,
    /// Check for driven internal signals never read by any output, memory, or property.
    pub check_unread: bool,
    /// Automatically discover FSM state registers and run deadlock/livelock/unreachable checks.
    pub auto_fsm: bool,
    /// Optional explicit FSM state register names to check for deadlock, livelock, and unreachable states.
    pub fsm_signals: Vec<String>,
    /// Optional allowlist of categories to run (empty = run all configured categories).
    pub enabled_categories: Vec<String>,
    /// Optional denylist / waiver list of categories to skip.
    pub disabled_categories: Vec<String>,
    pub use_pdr: bool,
}

impl Default for AutoCheckConfig {
    fn default() -> Self {
        Self {
            depth: 15,
            solver: "z3".to_string(),
            check_overflow: false,
            check_truncation: false,
            check_const_regs: false,
            check_no_reset: false,
            check_unread: false,
            auto_fsm: false,
            fsm_signals: Vec::new(),
            enabled_categories: Vec::new(),
            disabled_categories: Vec::new(),
            use_pdr: true,
        }
    }
}

impl AutoCheckConfig {
    /// Strict preset enabling all opt-in checks (overflow, truncation, const-reg, no-reset, unread, auto-FSM).
    pub fn strict() -> Self {
        Self {
            check_overflow: true,
            check_truncation: true,
            check_const_regs: true,
            check_no_reset: true,
            check_unread: true,
            auto_fsm: true,
            ..Default::default()
        }
    }

    /// Returns true if `category` is permitted by `enabled_categories` and `disabled_categories`.
    pub fn is_category_enabled(&self, category: &str) -> bool {
        if self
            .disabled_categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(category))
        {
            return false;
        }
        if self.enabled_categories.is_empty() {
            return true;
        }
        self.enabled_categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(category))
    }
}

#[derive(Debug, Clone)]
pub enum CheckStatus {
    Safe,
    Bug { step: usize, trace: Vec<TraceFrame> },
    Warning(String),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct AutoCheckFinding {
    pub category: String,
    pub name: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
}

/// Questa AutoCheck / Jasper Superlint equivalent: push-button formal RTL bug hunting requiring
/// ZERO SVA or testbench. Automatically checks:
/// - Out-of-bounds memory/array indexing (`oob_memory`)
/// - Dynamic bit-select & indexed part-select out-of-bounds (`oob_bitselect`)
/// - Divide/modulo by zero (`div_by_zero`)
/// - Out-of-range shift amounts (`shift_overflow`)
/// - `unique` / `unique0` / `priority` `case` & `if` overlap and completeness (`unique_priority`)
/// - Formal reachability-backed inferred latch detection in `always_comb` (`inferred_latch`)
/// - Combinational feedback loops (`comb_loop`) and multi-driver contention (`multi_driven`)
/// - Optional signed & unsigned add/sub/mul arithmetic overflow/underflow (`overflow`)
/// - Optional formal implicit width-truncation / data-loss detection (`width_truncation`)
/// - FSM deadlock, livelock, and unreachable declared states (explicit `--fsm` or `--auto-fsm`)
/// - Structural lint (undriven internal wires, clock-used-as-data, optional unread signals)
/// - Optional constant-stuck register (`const_reg`) and unresettable register (`no_reset_reg`) checks
pub fn run_autocheck(
    compile: &CompileOptions,
    cfg: &AutoCheckConfig,
) -> Result<(Module, Vec<AutoCheckFinding>)> {
    let inst = InstrumentOptions {
        branch_covers: false,
        unique_priority_checks: cfg.is_category_enabled("unique_priority"),
        oob_memory_checks: cfg.is_category_enabled("oob_memory"),
        oob_bitselect_checks: cfg.is_category_enabled("oob_bitselect"),
        div_zero_checks: cfg.is_category_enabled("div_by_zero"),
        overflow_checks: cfg.check_overflow && cfg.is_category_enabled("overflow"),
        shift_overflow_checks: cfg.is_category_enabled("shift_overflow"),
        latch_checks: cfg.is_category_enabled("inferred_latch"),
        truncation_checks: cfg.check_truncation && cfg.is_category_enabled("width_truncation"),
        multi_driver_checks: cfg.is_category_enabled("multi_driven"),
        allow_comb_loops: cfg.is_category_enabled("comb_loop"),
    };
    let mut m = compile_to_ir_with_options(compile, inst)?;

    let mut findings = Vec::new();

    // 0a. Surface structural lints captured during Slang lowering (`comb_loop`, `multi_driven`).
    let mut comb_loop_sigs: HashSet<String> = HashSet::new();
    for lint in &m.lints {
        if !cfg.is_category_enabled(&lint.category) {
            continue;
        }
        if lint.category == "comb_loop" {
            comb_loop_sigs.insert(lint.signal.clone());
        }
        findings.push(AutoCheckFinding {
            category: lint.category.clone(),
            name: lint.signal.clone(),
            status: if lint.is_bug {
                CheckStatus::Bug {
                    step: 0,
                    trace: Vec::new(),
                }
            } else {
                CheckStatus::Warning(lint.message.clone())
            },
            detail: Some(lint.message.clone()),
        });
    }

    // 0b. Structural clock-used-as-data check (MUST run BEFORE clk2fflogic!).
    if cfg.is_category_enabled("clock_as_data") {
        let clock_sigs: HashSet<SignalId> = m
            .signals
            .iter()
            .filter(|s| s.is_clock)
            .map(|s| s.id)
            .chain(m.ffs.iter().filter_map(|f| f.clock.map(|c| c.signal)))
            .collect();
        if !clock_sigs.is_empty() {
            let mut data_used_clocks: HashSet<SignalId> = HashSet::new();
            for ca in &m.comb {
                if !clock_sigs.contains(&ca.signal) {
                    collect_used_clocks(&m, ca.value, &clock_sigs, &mut data_used_clocks);
                }
            }
            for ff in &m.ffs {
                collect_used_clocks(&m, ff.next, &clock_sigs, &mut data_used_clocks);
            }
            for csid in data_used_clocks {
                let cname = &m.signal(csid).name;
                if !cname.starts_with('$') {
                    let msg = format!(
                        "clock signal '{cname}' is referenced inside data/combinational logic"
                    );
                    findings.push(AutoCheckFinding {
                        category: "clock_as_data".into(),
                        name: cname.clone(),
                        status: CheckStatus::Warning(msg.clone()),
                        detail: Some(msg),
                    });
                }
            }
        }
    }

    if vf_ir::passes::has_multiple_clocks(&m) {
        vf_ir::passes::clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        vf_ir::passes::async2sync(&mut m);
    }

    // 1. Structural lint: detect undriven internal wires/outputs.
    let driven: HashSet<_> = m
        .comb
        .iter()
        .map(|c| c.signal)
        .chain(m.ffs.iter().map(|f| f.signal))
        .collect();
    if cfg.is_category_enabled("undriven") {
        for s in &m.signals {
            if matches!(s.kind, SignalKind::Output | SignalKind::Wire)
                && !s.name.starts_with('$')
                && !driven.contains(&s.id)
                && !comb_loop_sigs.contains(&s.name)
            {
                let msg = format!("signal '{}' has no driver", s.name);
                findings.push(AutoCheckFinding {
                    category: "undriven".into(),
                    name: s.name.clone(),
                    status: CheckStatus::Warning(msg.clone()),
                    detail: Some(msg),
                });
            }
        }
    }

    // 1b. Optional unread/dead driven signal lint.
    if cfg.check_unread && cfg.is_category_enabled("unread_signal") {
        for sname in find_unread_signals(&m, &driven) {
            let msg = format!(
                "driven internal signal '{sname}' never reaches any output, memory, or property"
            );
            findings.push(AutoCheckFinding {
                category: "unread_signal".into(),
                name: sname,
                status: CheckStatus::Warning(msg.clone()),
                detail: Some(msg),
            });
        }
    }

    // 2. Check each auto-generated formal assertion in parallel with COI pruning + sequential sweeping!
    let auto_asserts: Vec<(String, Property)> = m
        .properties
        .iter()
        .filter(|p| p.kind == PropertyKind::Assert && p.name.starts_with("autocheck_"))
        .map(|p| (classify_autocheck_prop(&p.name).to_string(), p.clone()))
        .filter(|(cat, _)| cfg.is_category_enabled(cat))
        .collect();

    if !auto_asserts.is_empty() {
        let n_checks = auto_asserts.len();
        let workers = thread::available_parallelism()
            .map(|n| n.get().clamp(1, 4))
            .unwrap_or(2)
            .min(n_checks);
        let cursor = AtomicUsize::new(0);
        let results: Mutex<Vec<(usize, Result<AutoCheckFinding>)>> =
            Mutex::new(Vec::with_capacity(n_checks));

        thread::scope(|scope| {
            for _ in 0..workers {
                let m_ref = &m;
                let cfg_ref = cfg;
                let asserts_ref = &auto_asserts;
                let cursor_ref = &cursor;
                let results_ref = &results;
                scope.spawn(move || loop {
                    let idx = cursor_ref.fetch_add(1, Ordering::Relaxed);
                    if idx >= asserts_ref.len() {
                        break;
                    }
                    let (ref category, ref prop) = asserts_ref[idx];
                    let mut sub = m_ref.clone();
                    sub.properties.retain(|p| p.kind == PropertyKind::Assume);
                    sub.properties.push(prop.clone());
                    vf_ir::passes::coi_prune(&mut sub);
                    vf_ir::opt::sweep_sequential(&mut sub);
                    vf_ir::passes::coi_prune(&mut sub);

                    let finding = verify_single_assert(&sub, cfg_ref).map(|status| {
                        AutoCheckFinding {
                            category: category.clone(),
                            name: prop.name.clone(),
                            status,
                            detail: None,
                        }
                    });
                    if let Ok(mut guard) = results_ref.lock() {
                        guard.push((idx, finding));
                    }
                });
            }
        });

        let mut ordered = results.into_inner().unwrap_or_default();
        ordered.sort_by_key(|(idx, _)| *idx);
        for (_, res) in ordered {
            findings.push(res?);
        }
    }

    // 3. Optional constant-stuck register check: prove if any initialized user FF can never change.
    if cfg.check_const_regs && cfg.is_category_enabled("const_reg") {
        for ff in &m.ffs {
            let s = m.signal(ff.signal);
            if s.name.starts_with('$') || s.is_clock {
                continue;
            }
            if let Some(init) = &ff.init {
                let mut sub = m.clone();
                sub.properties.retain(|p| p.kind == PropertyKind::Assume);
                let se = sub.sig_expr(ff.signal);
                let ce = sub.c_bits(init.clone());
                let eq = sub.eq(se, ce, false);
                let one = sub.c_bool(true);
                sub.properties.push(Property {
                    name: format!("const_reg_{}", s.name),
                    kind: PropertyKind::Assert,
                    cond: eq,
                    en: one,
                    source: None,
                });
                vf_ir::passes::coi_prune(&mut sub);
                if let Ok(ProveOutcome::Proved { .. }) = run_kinduction(
                    &sub,
                    &ProveConfig {
                        max_k: 2,
                        solver: cfg.solver.clone(),
                    },
                ) {
                    let msg = format!(
                        "register '{}' is provably stuck constant at {}'b{}",
                        s.name, init.width, init.bits
                    );
                    findings.push(AutoCheckFinding {
                        category: "const_reg".into(),
                        name: s.name.clone(),
                        status: CheckStatus::Warning(msg.clone()),
                        detail: Some(msg),
                    });
                }
            }
        }
    }

    // 3b. Optional formal unresettable register check (`no_reset_reg`).
    if cfg.check_no_reset && cfg.is_category_enabled("no_reset_reg") {
        findings.extend(check_unresettable_regs(&m, cfg)?);
    }

    // 4. FSM analysis (deadlock, livelock, and unreachable declared states) for explicit `--fsm` and optional `--auto-fsm`.
    let mut fsm_targets: Vec<String> = cfg.fsm_signals.clone();
    if cfg.auto_fsm {
        for cand in discover_fsm_candidates(&m) {
            if !fsm_targets.contains(&cand) {
                fsm_targets.push(cand);
            }
        }
    }
    for fsm_name in &fsm_targets {
        if let Some(sig) = m.find_signal(fsm_name) {
            if let Some(ff) = m.ffs.iter().find(|f| f.signal == sig).cloned() {
                if cfg.is_category_enabled("fsm_deadlock") {
                    let dl_status = check_fsm_deadlock(&m, sig, ff.next, cfg)?;
                    findings.push(AutoCheckFinding {
                        category: "fsm_deadlock".into(),
                        name: fsm_name.clone(),
                        status: dl_status,
                        detail: None,
                    });
                }

                let declared_states = extract_fsm_states(&m, sig, ff.next);
                if !declared_states.is_empty() {
                    if cfg.is_category_enabled("fsm_unreachable") {
                        let unreach =
                            check_fsm_unreachable(&m, sig, fsm_name, &declared_states, cfg)?;
                        findings.extend(unreach);
                    }

                    if cfg.is_category_enabled("fsm_livelock") {
                        let ll_status = check_fsm_livelock(
                            &m,
                            sig,
                            ff.next,
                            ff.init.as_ref(),
                            &declared_states,
                            cfg,
                        )?;
                        findings.push(AutoCheckFinding {
                            category: "fsm_livelock".into(),
                            name: fsm_name.clone(),
                            status: ll_status,
                            detail: None,
                        });
                    }
                }
            }
        }
    }

    Ok((m, findings))
}

fn classify_autocheck_prop(name: &str) -> &'static str {
    if name.contains("oob_mem") {
        "oob_memory"
    } else if name.contains("oob_bitselect") {
        "oob_bitselect"
    } else if name.contains("div_by_zero") {
        "div_by_zero"
    } else if name.contains("shift_overflow") {
        "shift_overflow"
    } else if name.contains("inferred_latch") {
        "inferred_latch"
    } else if name.contains("width_truncation") {
        "width_truncation"
    } else if name.contains("overflow") {
        "overflow"
    } else {
        "unique_priority"
    }
}

/// Automatically discover FSM state register candidates (small-width FFs whose name hints at
/// FSM/state/mode OR that are compared against >= 2 distinct constants in equality checks).
pub fn discover_fsm_candidates(m: &Module) -> Vec<String> {
    let mut out = Vec::new();
    for ff in &m.ffs {
        let s = m.signal(ff.signal);
        if s.name.starts_with('$') || s.is_clock || s.width > 8 {
            continue;
        }
        let low = s.name.to_ascii_lowercase();
        let name_hint = low.contains("state")
            || low.contains("fsm")
            || low.ends_with("_st")
            || low == "st"
            || low == "mode";
        let mut const_cmps: HashSet<String> = HashSet::new();
        for node in &m.exprs {
            if let ExprKind::Eq(a, b) = &node.kind {
                match (&m.expr(*a).kind, &m.expr(*b).kind) {
                    (ExprKind::Signal(sig), ExprKind::Const(c))
                    | (ExprKind::Const(c), ExprKind::Signal(sig))
                        if *sig == ff.signal && c.width == s.width =>
                    {
                        const_cmps.insert(c.bits.clone());
                    }
                    _ => {}
                }
            }
        }
        if name_hint || const_cmps.len() >= 2 {
            out.push(s.name.clone());
        }
    }
    out
}

/// Find driven internal wires/registers that never appear in the backward cone of influence
/// of any output port, memory write, or user property.
fn find_unread_signals(m: &Module, driven: &HashSet<SignalId>) -> Vec<String> {
    let comb_map: std::collections::HashMap<SignalId, ExprId> =
        m.comb.iter().map(|c| (c.signal, c.value)).collect();
    let ff_map: std::collections::HashMap<SignalId, usize> = m
        .ffs
        .iter()
        .enumerate()
        .map(|(i, f)| (f.signal, i))
        .collect();

    let mut live: HashSet<SignalId> = HashSet::new();
    let mut worklist: Vec<SignalId> = Vec::new();
    let enqueue_expr = |eid: ExprId, live: &mut HashSet<SignalId>, wl: &mut Vec<SignalId>| {
        let mut deps = Vec::new();
        collect_signals(m, eid, &mut deps);
        for d in deps {
            if live.insert(d) {
                wl.push(d);
            }
        }
    };

    for s in &m.signals {
        if s.kind == SignalKind::Output && live.insert(s.id) {
            worklist.push(s.id);
        }
    }
    for mw in &m.mem_writes {
        enqueue_expr(mw.addr, &mut live, &mut worklist);
        enqueue_expr(mw.data, &mut live, &mut worklist);
        enqueue_expr(mw.enable, &mut live, &mut worklist);
    }
    for p in &m.properties {
        if !p.name.starts_with("autocheck_") {
            enqueue_expr(p.cond, &mut live, &mut worklist);
            enqueue_expr(p.en, &mut live, &mut worklist);
        }
    }

    while let Some(sig) = worklist.pop() {
        if let Some(&val) = comb_map.get(&sig) {
            enqueue_expr(val, &mut live, &mut worklist);
        }
        if let Some(&fi) = ff_map.get(&sig) {
            let ff = &m.ffs[fi];
            enqueue_expr(ff.next, &mut live, &mut worklist);
            if let Some(ar) = &ff.async_reset {
                enqueue_expr(ar.cond, &mut live, &mut worklist);
                enqueue_expr(ar.reset_val, &mut live, &mut worklist);
            }
        }
    }

    let mut unread = Vec::new();
    for s in &m.signals {
        if !s.name.starts_with('$')
            && !s.is_clock
            && matches!(s.kind, SignalKind::Wire | SignalKind::Reg)
            && driven.contains(&s.id)
            && !live.contains(&s.id)
        {
            unread.push(s.name.clone());
        }
    }
    unread
}

/// Detect standard reset input (`rst`/`rst_n`/`reset`/`reset_n`/`presetn`) and formally check
/// whether any uninitialized user FF fails to be cleared to a fixed value when reset is active.
fn check_unresettable_regs(m: &Module, cfg: &AutoCheckConfig) -> Result<Vec<AutoCheckFinding>> {
    use vf_smt::smt2;
    use vf_smt::solver::{SatResult, SmtSolver};

    let rst_info: Option<(SignalId, String, bool)> =
        ["rst", "rst_n", "reset", "reset_n", "presetn"]
            .iter()
            .find_map(|name| {
                m.find_signal(name).and_then(|sid| {
                    if m.signal(sid).kind == SignalKind::Input && m.signal(sid).width == 1 {
                        let low = name.ends_with("_n") || name.ends_with("resetn");
                        Some((sid, (*name).to_string(), low))
                    } else {
                        None
                    }
                })
            });

    let Some((rst_sig, rst_name, active_low)) = rst_info else {
        return Ok(Vec::new());
    };
    let active_val = if active_low { "#b0" } else { "#b1" };

    let mut out = Vec::new();
    for ff in &m.ffs {
        let s = m.signal(ff.signal);
        if s.name.starts_with('$') || s.is_clock || ff.init.is_some() || ff.async_reset.is_some() {
            continue;
        }
        // Check if both (cur == 0 && next == 0) and (cur == 1 && next == 1) are SAT under reset active.
        // Any FF reset to a constant C will make at least one of these UNSAT!
        let mut holds_zero = false;
        let mut holds_one = false;
        let w = s.width;
        let c0 = ConstBits::zero(w);
        let c1 = ConstBits::one(w);

        for (target_c, slot) in [(&c0, &mut holds_zero), (&c1, &mut holds_one)] {
            let mut solver = SmtSolver::spawn(&cfg.solver)?;
            solver.send(&smt2::header(m))?;
            solver.send(&smt2::encode_step(m, 0, false, None))?;
            solver.send(&smt2::encode_assumes(m, 0))?;
            solver.send(&smt2::encode_step(m, 1, false, Some(0)))?;
            solver.send(&format!(
                "(assert (= {} {active_val}))\n",
                smt2::sig_name(rst_sig, 0)
            ))?;
            let tc = smt2::smt_const(target_c);
            solver.send(&format!(
                "(assert (= {} {tc}))\n(assert (= {} {tc}))\n",
                smt2::sig_name(ff.signal, 0),
                smt2::sig_name(ff.signal, 1)
            ))?;
            if solver.check_sat()? == SatResult::Sat {
                *slot = true;
            }
        }

        if holds_zero && holds_one {
            let msg = format!(
                "uninitialized register '{}' is not cleared when reset '{rst_name}' is active",
                s.name
            );
            out.push(AutoCheckFinding {
                category: "no_reset_reg".into(),
                name: s.name.clone(),
                status: CheckStatus::Warning(msg.clone()),
                detail: Some(msg),
            });
        }
    }
    Ok(out)
}

fn collect_used_clocks(
    m: &Module,
    root: ExprId,
    clocks: &HashSet<SignalId>,
    found: &mut HashSet<SignalId>,
) {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(eid) = stack.pop() {
        if !visited.insert(eid) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Signal(sid) => {
                if clocks.contains(sid) {
                    found.insert(*sid);
                }
            }
            ExprKind::Const(_) => {}
            ExprKind::Not(a)
            | ExprKind::Neg(a)
            | ExprKind::RedAnd(a)
            | ExprKind::RedOr(a)
            | ExprKind::RedXor(a)
            | ExprKind::Slice { expr: a, .. }
            | ExprKind::Zext { expr: a, .. }
            | ExprKind::Sext { expr: a, .. }
            | ExprKind::MemRead { addr: a, .. } => stack.push(*a),
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
                stack.push(*a);
                stack.push(*b);
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                stack.push(*cond);
                stack.push(*then_val);
                stack.push(*else_val);
            }
            ExprKind::Concat(parts) => stack.extend(parts.iter().copied()),
        }
    }
}

/// Extract declared FSM state encodings referenced by equality checks against `sig` or
/// constant assignments inside `next_expr`.
fn extract_fsm_states(m: &Module, sig: SignalId, next_expr: ExprId) -> Vec<ConstBits> {
    let w = m.signal(sig).width;
    let mut vals: HashSet<ConstBits> = HashSet::new();

    // 1. Constants compared against `sig` anywhere in the module's expressions (`fsm == STATE`).
    for node in &m.exprs {
        if let ExprKind::Eq(a, b) = &node.kind {
            match (&m.expr(*a).kind, &m.expr(*b).kind) {
                (ExprKind::Signal(s), ExprKind::Const(c))
                | (ExprKind::Const(c), ExprKind::Signal(s))
                    if *s == sig && c.width == w =>
                {
                    vals.insert(c.clone());
                }
                _ => {}
            }
        }
    }

    // 2. Constants assigned to `sig` as leaves of `Mux` trees in `next_expr`.
    let mut stack = vec![next_expr];
    let mut seen = HashSet::new();
    while let Some(eid) = stack.pop() {
        if !seen.insert(eid) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Const(c) if c.width == w => {
                vals.insert(c.clone());
            }
            ExprKind::Mux {
                then_val, else_val, ..
            } => {
                stack.push(*then_val);
                stack.push(*else_val);
            }
            _ => {}
        }
    }

    if let Some(init) = &m.signal(sig).init {
        if init.width == w {
            vals.insert(init.clone());
        }
    }

    let mut sorted: Vec<ConstBits> = vals.into_iter().collect();
    sorted.sort_by(|a, b| a.bits.cmp(&b.bits));
    sorted.truncate(32);
    sorted
}

/// Check every declared FSM state encoding for reachability; prove unbounded unreachability for dead states.
fn check_fsm_unreachable(
    m: &Module,
    sig: SignalId,
    fsm_name: &str,
    states: &[ConstBits],
    cfg: &AutoCheckConfig,
) -> Result<Vec<AutoCheckFinding>> {
    let mut sub = m.clone();
    sub.properties.retain(|p| p.kind == PropertyKind::Assume);
    let sig_e = sub.sig_expr(sig);
    let one = sub.c_bool(true);
    for st in states {
        let ce = sub.c_bits(st.clone());
        let eq = sub.eq(sig_e, ce, false);
        sub.properties.push(Property {
            name: format!("fsm_state_{}", st.bits),
            kind: PropertyKind::Cover,
            cond: eq,
            en: one,
            source: None,
        });
    }

    let cover_res = run_cover(
        &sub,
        &CoverConfig {
            depth: cfg.depth,
            solver: cfg.solver.clone(),
            append: 0,
        },
    )?;

    let mut out = Vec::new();
    let mut any_unreach = false;
    for (st, res) in states.iter().zip(cover_res.iter()) {
        if res.reached.is_none() {
            // Confirm unbounded unreachability via k-induction/PDR (`assert (sig != st)`).
            let mut prove_mod = m.clone();
            prove_mod
                .properties
                .retain(|p| p.kind == PropertyKind::Assume);
            let se = prove_mod.sig_expr(sig);
            let ce = prove_mod.c_bits(st.clone());
            let neq = prove_mod.intern(1, ExprKind::Ne(se, ce));
            let one_p = prove_mod.c_bool(true);
            prove_mod.properties.push(Property {
                name: format!("fsm_unreach_{}", st.bits),
                kind: PropertyKind::Assert,
                cond: neq,
                en: one_p,
                source: None,
            });
            vf_ir::passes::coi_prune(&mut prove_mod);
            if matches!(verify_single_assert(&prove_mod, cfg)?, CheckStatus::Safe) {
                any_unreach = true;
                let label = st
                    .to_u64()
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| format!("{}'b{}", st.width, st.bits));
                let msg = format!(
                    "declared FSM state '{fsm_name} == {label}' is formally unreachable"
                );
                out.push(AutoCheckFinding {
                    category: "fsm_unreachable".into(),
                    name: format!("{fsm_name}=={label}"),
                    status: CheckStatus::Warning(msg.clone()),
                    detail: Some(msg),
                });
            }
        }
    }

    if !any_unreach {
        out.push(AutoCheckFinding {
            category: "fsm_unreachable".into(),
            name: fsm_name.to_string(),
            status: CheckStatus::Safe,
            detail: None,
        });
    }

    Ok(out)
}

/// Detect multi-state FSM livelock: a reachable non-initial state `s != init` where the FSM can
/// still transition (`next != cur`, so not a single-state deadlock) yet CANNOT return to `init`
/// within `depth` cycles without asserting reset.
fn check_fsm_livelock(
    m: &Module,
    sig: SignalId,
    next_expr: ExprId,
    init_opt: Option<&ConstBits>,
    states: &[ConstBits],
    cfg: &AutoCheckConfig,
) -> Result<CheckStatus> {
    use vf_smt::smt2;
    use vf_smt::solver::{SatResult, SmtSolver};

    let w = m.signal(sig).width;
    let init_val = init_opt
        .cloned()
        .unwrap_or_else(|| ConstBits::zero(w));

    // Detect optional reset input so livelock tests escape WITHOUT relying on hardware reset.
    let rst_info: Option<(SignalId, bool)> = ["rst", "rst_n", "reset", "reset_n", "presetn"]
        .iter()
        .find_map(|name| {
            m.find_signal(name).and_then(|sid| {
                if m.signal(sid).kind == SignalKind::Input && m.signal(sid).width == 1 {
                    let low = name.ends_with("_n") || name.ends_with("resetn");
                    Some((sid, low))
                } else {
                    None
                }
            })
        });

    let mut dm = m.clone();
    let cur_e = dm.sig_expr(sig);
    let moving_e = dm.intern(1, ExprKind::Ne(next_expr, cur_e));
    let moving_sig = dm.add_signal("$fsm_moving", 1, false, SignalKind::Wire);
    dm.comb.push(vf_ir::CombAssign {
        signal: moving_sig,
        value: moving_e,
    });
    let _ = dm.topo_sort_comb();

    // Find shortest trace reaching each non-init declared state (with reset inactive at arrival).
    let escape_depth = cfg.depth.clamp(4, 12);
    for st in states {
        if st == &init_val {
            continue;
        }
        // Check if state `st` is reachable.
        let mut cov_m = dm.clone();
        cov_m.properties.retain(|p| p.kind == PropertyKind::Assume);
        let se = cov_m.sig_expr(sig);
        let ce = cov_m.c_bits(st.clone());
        let eq = cov_m.eq(se, ce, false);
        let en = if let Some((rsid, active_low)) = rst_info {
            let re = cov_m.sig_expr(rsid);
            if active_low {
                re
            } else {
                cov_m.logic_not(re)
            }
        } else {
            cov_m.c_bool(true)
        };
        cov_m.properties.push(Property {
            name: "reach_st".into(),
            kind: PropertyKind::Cover,
            cond: eq,
            en,
            source: None,
        });
        let cov = run_cover(
            &cov_m,
            &CoverConfig {
                depth: cfg.depth,
                solver: cfg.solver.clone(),
                append: 0,
            },
        )?;
        let Some((reach_step, trace)) = cov.into_iter().next().and_then(|r| r.reached) else {
            continue;
        };

        // First verify it is NOT a single-state deadlock (i.e. `moving == 1` is possible from `trace[reach_step]`).
        let mut mv = SmtSolver::spawn(&cfg.solver)?;
        mv.send(&smt2::header(&dm))?;
        mv.send(&smt2::encode_step(&dm, 0, false, None))?;
        mv.send(&smt2::encode_assumes(&dm, 0))?;
        for ff in &m.ffs {
            let fname = &m.signal(ff.signal).name;
            if let Some((_, v)) = trace[reach_step].values.iter().find(|(n, _)| n == fname) {
                mv.send(&format!(
                    "(assert (= {} {}))\n",
                    smt2::sig_name(ff.signal, 0),
                    smt2::smt_const(v)
                ))?;
            }
        }
        if let Some((rsid, active_low)) = rst_info {
            let inactive = if active_low { "#b1" } else { "#b0" };
            mv.send(&format!(
                "(assert (= {} {inactive}))\n",
                smt2::sig_name(rsid, 0)
            ))?;
        }
        mv.send(&format!(
            "(assert (= {} #b1))\n",
            smt2::sig_name(moving_sig, 0)
        ))?;
        if mv.check_sat()? != SatResult::Sat {
            // Single-state deadlock (already caught by `fsm_deadlock`).
            continue;
        }

        // Now test if ANY input sequence (with reset inactive) can return `sig` to `init_val` within `escape_depth` steps.
        let mut esc = SmtSolver::spawn(&cfg.solver)?;
        esc.send(&smt2::header(&dm))?;
        esc.send(&smt2::encode_step(&dm, 0, false, None))?;
        esc.send(&smt2::encode_assumes(&dm, 0))?;
        for ff in &m.ffs {
            let fname = &m.signal(ff.signal).name;
            if let Some((_, v)) = trace[reach_step].values.iter().find(|(n, _)| n == fname) {
                esc.send(&format!(
                    "(assert (= {} {}))\n",
                    smt2::sig_name(ff.signal, 0),
                    smt2::smt_const(v)
                ))?;
            }
        }
        if let Some((rsid, active_low)) = rst_info {
            let inactive = if active_low { "#b1" } else { "#b0" };
            esc.send(&format!(
                "(assert (= {} {inactive}))\n",
                smt2::sig_name(rsid, 0)
            ))?;
        }

        let init_smt = smt2::smt_const(&init_val);
        let mut can_return = false;
        for step in 1..=escape_depth {
            esc.send(&smt2::encode_step(&dm, step, false, Some(step - 1)))?;
            esc.send(&smt2::encode_assumes(&dm, step))?;
            if let Some((rsid, active_low)) = rst_info {
                let inactive = if active_low { "#b1" } else { "#b0" };
                esc.send(&format!(
                    "(assert (= {} {inactive}))\n",
                    smt2::sig_name(rsid, step)
                ))?;
            }
            esc.push()?;
            esc.send(&format!(
                "(assert (= {} {init_smt}))\n",
                smt2::sig_name(sig, step)
            ))?;
            if esc.check_sat()? == SatResult::Sat {
                can_return = true;
                esc.pop()?;
                break;
            }
            esc.pop()?;
        }

        if !can_return {
            return Ok(CheckStatus::Bug {
                step: reach_step,
                trace,
            });
        }
    }

    Ok(CheckStatus::Safe)
}

fn verify_single_assert(m: &Module, cfg: &AutoCheckConfig) -> Result<CheckStatus> {
    let k = run_kinduction(
        m,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )?;
    match k {
        ProveOutcome::Proved { .. } => Ok(CheckStatus::Safe),
        ProveOutcome::Fail { step, trace, .. } => Ok(CheckStatus::Bug { step, trace }),
        ProveOutcome::AssumptionsUnsat { .. } => Ok(CheckStatus::Safe),
        ProveOutcome::Unknown { .. } => {
            if cfg.use_pdr {
                if let Ok(p) = run_pdr(
                    m,
                    &PdrConfig {
                        smt_solver: cfg.solver.clone(),
                        ..Default::default()
                    },
                ) {
                    return match p {
                        ProveOutcome::Proved { .. } => Ok(CheckStatus::Safe),
                        ProveOutcome::Fail { step, trace, .. } => {
                            Ok(CheckStatus::Bug { step, trace })
                        }
                        _ => Ok(CheckStatus::Unknown),
                    };
                }
            }
            Ok(CheckStatus::Unknown)
        }
    }
}

/// Detect whether an FSM register `sig` has a reachable deadlock state where `next == cur`
/// regardless of primary inputs (i.e. even when trying to escape, `next(s, in) == s` for all `in`).
fn check_fsm_deadlock(
    m: &Module,
    sig: vf_ir::SignalId,
    next_expr: vf_ir::ExprId,
    cfg: &AutoCheckConfig,
) -> Result<CheckStatus> {
    use vf_smt::smt2;
    use vf_smt::solver::{SatResult, SmtSolver};

    let inputs: Vec<_> = m
        .signals
        .iter()
        .filter(|s| s.kind == SignalKind::Input && !s.is_clock)
        .collect();

    let rst_info: Option<(SignalId, bool)> = ["rst", "rst_n", "reset", "reset_n", "presetn"]
        .iter()
        .find_map(|name| {
            m.find_signal(name).and_then(|sid| {
                if m.signal(sid).kind == SignalKind::Input && m.signal(sid).width == 1 {
                    let low = name.ends_with("_n") || name.ends_with("resetn");
                    Some((sid, low))
                } else {
                    None
                }
            })
        });

    let mut solver = SmtSolver::spawn(&cfg.solver)?;
    solver.send("(set-logic ALL)\n(set-option :produce-models true)\n")?;

    let mut dm = m.clone();
    let cur_e = dm.sig_expr(sig);
    let stuck_e = dm.intern(1, ExprKind::Eq(next_expr, cur_e));
    let stuck_sig = dm.add_signal("$fsm_stuck", 1, false, SignalKind::Wire);
    dm.comb.push(vf_ir::CombAssign {
        signal: stuck_sig,
        value: stuck_e,
    });
    let _ = dm.topo_sort_comb();

    for t in 0..cfg.depth {
        solver.send(&smt2::encode_step(&dm, t, t == 0, t.checked_sub(1)))?;
        solver.send(&smt2::encode_assumes(&dm, t))?;

        solver.push()?;
        solver.send(&format!(
            "(assert (= {} #b1))\n",
            smt2::sig_name(stuck_sig, t)
        ))?;
        let mut blocked_states: Vec<String> = Vec::new();
        loop {
            match solver.check_sat()? {
                SatResult::Sat => {
                    let trace = solver.extract_trace(&dm, t + 1)?;
                    let cur_val = trace[t]
                        .values
                        .iter()
                        .find(|(n, _)| n == &m.signal(sig).name)
                        .map(|(_, v)| v.clone())
                        .unwrap();

                    let mut escape = SmtSolver::spawn(&cfg.solver)?;
                    escape.send(&smt2::header(&dm))?;
                    escape.send(&smt2::encode_step(&dm, 0, false, None))?;
                    escape.send(&smt2::encode_assumes(&dm, 0))?;
                    for ff in &m.ffs {
                        let fname = &m.signal(ff.signal).name;
                        if let Some((_, v)) = trace[t].values.iter().find(|(n, _)| n == fname) {
                            escape.send(&format!(
                                "(assert (= {} {}))\n",
                                smt2::sig_name(ff.signal, 0),
                                smt2::smt_const(v)
                            ))?;
                        }
                    }
                    if let Some((rsid, active_low)) = rst_info {
                        let inactive = if active_low { "#b1" } else { "#b0" };
                        escape.send(&format!(
                            "(assert (= {} {inactive}))\n",
                            smt2::sig_name(rsid, 0)
                        ))?;
                    }
                    escape.send(&format!(
                        "(assert (= {} #b0))\n",
                        smt2::sig_name(stuck_sig, 0)
                    ))?;
                    let can_escape = escape.check_sat()?;
                    if can_escape == SatResult::Unsat {
                        solver.pop()?;
                        return Ok(CheckStatus::Bug { step: t, trace });
                    }
                    let val_smt = smt2::smt_const(&cur_val);
                    if blocked_states.len() > 32 || inputs.is_empty() {
                        break;
                    }
                    blocked_states.push(val_smt.clone());
                    solver.send(&format!(
                        "(assert (not (= {} {val_smt})))\n",
                        smt2::sig_name(sig, t)
                    ))?;
                }
                _ => break,
            }
        }
        solver.pop()?;
    }

    Ok(CheckStatus::Safe)
}
