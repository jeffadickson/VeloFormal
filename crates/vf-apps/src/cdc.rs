use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{
    ClockEdge, ClockSpec, ConstBits, ExprId, ExprKind, Ff, Module, Property, PropertyKind,
    SignalId, SignalKind,
};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct CdcConfig {
    /// Signals that should be formally verified to be Gray-coded (`popcount(s ^ $past(s)) <= 1`).
    pub gray_signals: Vec<String>,
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for CdcConfig {
    fn default() -> Self {
        Self {
            gray_signals: Vec::new(),
            depth: 15,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdcSeverity {
    Error,
    Warning,
    Safe,
}

#[derive(Debug, Clone)]
pub struct CdcFinding {
    pub severity: CdcSeverity,
    pub kind: String,
    pub src_signal: String,
    pub src_domain: String,
    pub dst_signal: String,
    pub dst_domain: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub enum GrayStatus {
    Proved,
    Fail { step: usize, trace: Vec<TraceFrame> },
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GrayResult {
    pub signal: String,
    pub status: GrayStatus,
}

#[derive(Debug, Clone)]
pub struct CdcReport {
    pub clock_domains: Vec<String>,
    pub findings: Vec<CdcFinding>,
    pub gray_results: Vec<GrayResult>,
}

/// Questa CDC / RDC equivalent:
/// - Structural clock-domain crossing (CDC) analyzer: detects unsynchronized single-bit crossings,
///   multi-bit unsynchronized crossings, combinational logic in CDC paths, 2-FF synchronizers,
///   and CDC reconvergence.
/// - Structural reset-domain crossing (RDC) analyzer: detects async-reset FFs feeding non-async-reset
///   or differently-reset FFs.
/// - Formal Gray-code protocol verification (`$countones(sig ^ $past(sig)) <= 1`).
///
/// NOTE: Pass the raw elaborated `Module` BEFORE `clk2fflogic` and `async2sync` so every `Ff`
/// retains its exact `ClockSpec` and `AsyncReset`.
pub fn run_cdc_analysis(raw: &Module, cfg: &CdcConfig) -> Result<CdcReport> {
    let mut domains_set: HashSet<ClockSpec> = HashSet::new();
    let mut ff_by_sig: HashMap<SignalId, &Ff> = HashMap::new();
    for ff in &raw.ffs {
        ff_by_sig.insert(ff.signal, ff);
        if let Some(c) = ff.clock {
            domains_set.insert(c);
        }
    }

    let mut clock_domains: Vec<String> = domains_set
        .iter()
        .map(|c| fmt_domain(raw, Some(*c)))
        .collect();
    clock_domains.sort();

    let comb_map: HashMap<SignalId, ExprId> = raw
        .comb
        .iter()
        .map(|c| (c.signal, c.value))
        .collect();

    // Precompute fanout FFs for each FF signal so we can recognize 2-FF synchronizer chains!
    let mut ff_fanin_info: HashMap<SignalId, Vec<(SignalId, bool)>> = HashMap::new();
    for ff in &raw.ffs {
        let deps = collect_source_ffs(raw, &ff_by_sig, &comb_map, ff.next);
        ff_fanin_info.insert(ff.signal, deps);
    }

    // Identify 1-bit first-stage synchronizer FFs:
    // `sync1` has clock C_dst, width == 1, depends directly (no comb gates) on a single foreign FF `src`,
    // and ONLY feeds a second-stage FF `sync2` in the same clock domain C_dst (with no comb gates).
    let mut direct_ff_consumers: HashMap<SignalId, Vec<(SignalId, bool)>> = HashMap::new();
    for (&dst_sig, deps) in &ff_fanin_info {
        for &(src_sig, has_comb) in deps {
            direct_ff_consumers
                .entry(src_sig)
                .or_default()
                .push((dst_sig, has_comb));
        }
    }

    let mut is_sync_stage1: HashSet<SignalId> = HashSet::new();
    let mut sync_stage2_origin: HashMap<SignalId, ClockSpec> = HashMap::new();

    for ff in &raw.ffs {
        let Some(dst_clk) = ff.clock else { continue };
        if raw.signal(ff.signal).width != 1 {
            continue;
        }
        let deps = &ff_fanin_info[&ff.signal];
        if deps.len() == 1 && !deps[0].1 {
            let src_sig = deps[0].0;
            if let Some(src_ff) = ff_by_sig.get(&src_sig) {
                if src_ff.clock != ff.clock && raw.signal(src_sig).width == 1 {
                    // Check consumers of `ff.signal`: all FF consumers must be in `dst_clk` with no comb logic,
                    // and at least one such stage-2 FF must exist.
                    if let Some(consumers) = direct_ff_consumers.get(&ff.signal) {
                        let valid_stage2 = !consumers.is_empty()
                            && consumers.iter().all(|&(c_sig, c_comb)| {
                                !c_comb
                                    && ff_by_sig
                                        .get(&c_sig)
                                        .map(|cf| cf.clock == Some(dst_clk))
                                        .unwrap_or(false)
                            });
                        if valid_stage2 {
                            is_sync_stage1.insert(ff.signal);
                            if let Some(sc) = src_ff.clock {
                                for &(c_sig, _) in consumers {
                                    sync_stage2_origin.insert(c_sig, sc);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut findings = Vec::new();
    let gray_set: HashSet<&str> = cfg.gray_signals.iter().map(String::as_str).collect();

    // 1. CDC crossings & RDC crossings.
    for dst_ff in &raw.ffs {
        let dst_name = &raw.signal(dst_ff.signal).name;
        let dst_dom = fmt_domain(raw, dst_ff.clock);
        let deps = &ff_fanin_info[&dst_ff.signal];

        // Track synchronized sources feeding this FF for reconvergence check.
        let mut sync_sources_by_domain: HashMap<ClockSpec, Vec<String>> = HashMap::new();

        for &(src_sig, has_comb) in deps {
            if src_sig == dst_ff.signal {
                continue;
            }
            let Some(src_ff) = ff_by_sig.get(&src_sig) else {
                continue;
            };
            let src_name = &raw.signal(src_sig).name;
            let src_dom = fmt_domain(raw, src_ff.clock);

            // Check CDC reconvergence (multiple 2-FF synchronized signals from the same foreign clock).
            if let Some(&orig_clk) = sync_stage2_origin.get(&src_sig) {
                sync_sources_by_domain
                    .entry(orig_clk)
                    .or_default()
                    .push(src_name.clone());
            }

            // CDC check: different clocks.
            if src_ff.clock.is_some() && dst_ff.clock.is_some() && src_ff.clock != dst_ff.clock {
                let w = raw.signal(src_sig).width;
                if is_sync_stage1.contains(&dst_ff.signal) {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Safe,
                        kind: "synchronized_cdc".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom.clone(),
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: "1-bit crossing protected by 2-FF synchronizer".into(),
                    });
                } else if w > 1 && gray_set.contains(src_name.as_str()) {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Safe,
                        kind: "gray_coded_cdc".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom.clone(),
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: format!(
                            "{w}-bit crossing flagged as Gray-coded (checked formally below)"
                        ),
                    });
                } else if w > 1 {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Error,
                        kind: "multi_bit_cdc".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom.clone(),
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: format!(
                            "{w}-bit bus crosses clock domains without Gray-code/handshake sync"
                        ),
                    });
                } else if has_comb {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Error,
                        kind: "comb_logic_cdc".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom.clone(),
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: "combinational logic on CDC path causes glitch/metastability hazard"
                            .into(),
                    });
                } else {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Error,
                        kind: "unsynchronized_cdc".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom.clone(),
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: "1-bit CDC crossing lacks 2-FF synchronizer".into(),
                    });
                }
            }

            // RDC check: source FF has async reset, destination FF has different or no async reset.
            if let Some(src_ar) = &src_ff.async_reset {
                let same_reset = dst_ff
                    .async_reset
                    .as_ref()
                    .map(|dar| dar.cond == src_ar.cond)
                    .unwrap_or(false);
                if !same_reset {
                    findings.push(CdcFinding {
                        severity: CdcSeverity::Warning,
                        kind: "rdc_crossing".into(),
                        src_signal: src_name.clone(),
                        src_domain: src_dom,
                        dst_signal: dst_name.clone(),
                        dst_domain: dst_dom.clone(),
                        detail: "async-reset FF drives FF without matching async reset (RDC hazard)"
                            .into(),
                    });
                }
            }
        }

        for (orig_clk, srcs) in sync_sources_by_domain {
            if srcs.len() >= 2 {
                findings.push(CdcFinding {
                    severity: CdcSeverity::Warning,
                    kind: "cdc_reconvergence".into(),
                    src_signal: srcs.join(", "),
                    src_domain: fmt_domain(raw, Some(orig_clk)),
                    dst_signal: dst_name.clone(),
                    dst_domain: dst_dom.clone(),
                    detail: format!(
                        "{} independently synchronized signals reconverge (coherency hazard)",
                        srcs.len()
                    ),
                });
            }
        }
    }

    // 2. Formal Gray-code checks (`popcount(sig ^ $past(sig)) <= 1`).
    let mut gray_results = Vec::new();
    if !cfg.gray_signals.is_empty() {
        let mut prepared = raw.clone();
        if vf_ir::passes::has_multiple_clocks(&prepared) {
            vf_ir::passes::clk2fflogic(&mut prepared);
        }
        if prepared.ffs.iter().any(|f| f.async_reset.is_some()) {
            vf_ir::passes::async2sync(&mut prepared);
        }
        for sig_name in &cfg.gray_signals {
            let status = verify_gray_code(&prepared, sig_name, cfg)?;
            gray_results.push(GrayResult {
                signal: sig_name.clone(),
                status,
            });
        }
    }

    Ok(CdcReport {
        clock_domains,
        findings,
        gray_results,
    })
}

fn fmt_domain(m: &Module, clk: Option<ClockSpec>) -> String {
    match clk {
        Some(c) => {
            let edge = match c.edge {
                ClockEdge::PosEdge => "posedge",
                ClockEdge::NegEdge => "negedge",
            };
            format!("{edge} {}", m.signal(c.signal).name)
        }
        None => "implicit_clk".to_string(),
    }
}

/// Walk `expr` backwards through combinational wires to find all source FFs and whether
/// any combinational operator lies on the path (`has_comb`).
fn collect_source_ffs(
    m: &Module,
    ff_by_sig: &HashMap<SignalId, &Ff>,
    comb_map: &HashMap<SignalId, ExprId>,
    root: ExprId,
) -> Vec<(SignalId, bool)> {
    let mut result: HashMap<SignalId, bool> = HashMap::new();
    let mut visited_expr: HashSet<(ExprId, bool)> = HashSet::new();
    let mut stack: Vec<(ExprId, bool)> = vec![(root, false)];

    while let Some((eid, has_comb)) = stack.pop() {
        if !visited_expr.insert((eid, has_comb)) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Const(_) => {}
            ExprKind::Signal(sid) => {
                if ff_by_sig.contains_key(sid) {
                    result
                        .entry(*sid)
                        .and_modify(|v| *v |= has_comb)
                        .or_insert(has_comb);
                } else if let Some(&rhs) = comb_map.get(sid) {
                    stack.push((rhs, has_comb));
                }
            }
            ExprKind::Not(a)
            | ExprKind::Neg(a)
            | ExprKind::RedAnd(a)
            | ExprKind::RedOr(a)
            | ExprKind::RedXor(a) => stack.push((*a, true)),
            ExprKind::Zext { expr, .. }
            | ExprKind::Sext { expr, .. }
            | ExprKind::Slice { expr, .. } => stack.push((*expr, has_comb)),
            ExprKind::Concat(parts) => {
                for &p in parts {
                    stack.push((p, true));
                }
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                // Ignore simple synchronous-reset or clock-enable mux where one branch is constant
                // or self-hold? Keep accurate: a mux on a crossing path IS combinational logic
                // unless it's the FF's own reset/hold mux!
                stack.push((*cond, true));
                stack.push((*then_val, true));
                stack.push((*else_val, true));
            }
            ExprKind::MemRead { addr, .. } => stack.push((*addr, true)),
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
                stack.push((*a, true));
                stack.push((*b, true));
            }
        }
    }

    let mut v: Vec<(SignalId, bool)> = result.into_iter().collect();
    v.sort_by_key(|(s, _)| s.0);
    v
}

/// Formally verify that `sig_name` changes by at most 1 bit per cycle: `(diff & (diff - 1)) == 0`.
fn verify_gray_code(base: &Module, sig_name: &str, cfg: &CdcConfig) -> Result<GrayStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let sig = m
        .find_signal(sig_name)
        .ok_or_else(|| anyhow!("cdc gray-code signal '{sig_name}' not found"))?;
    let w = m.signal(sig).width;
    let cur = m.sig_expr(sig);

    let prev_sig = m.add_signal(format!("$gray_prev_{sig_name}"), w, false, SignalKind::Reg);
    m.ffs.push(Ff {
        signal: prev_sig,
        next: cur,
        init: None,
        clock: None,
        async_reset: None,
    });
    let prev = m.sig_expr(prev_sig);

    let valid_sig = m.add_signal("$gray_valid", 1, false, SignalKind::Reg);
    m.signal_mut(valid_sig).init = Some(ConstBits::zero(1));
    let one_b = m.c_bool(true);
    m.ffs.push(Ff {
        signal: valid_sig,
        next: one_b,
        init: Some(ConstBits::zero(1)),
        clock: None,
        async_reset: None,
    });
    let en = m.sig_expr(valid_sig);

    // At most 1 bit changed iff `diff & (diff - 1) == 0` where `diff = cur ^ prev`!
    let diff = m.intern(w, ExprKind::Xor(cur, prev));
    let one = m.c_u64(w, 1);
    let diff_m1 = m.intern(w, ExprKind::Sub(diff, one));
    let masked = m.intern(w, ExprKind::And(diff, diff_m1));
    let zero = m.c_bits(ConstBits::zero(w));
    let onehot0 = m.eq(masked, zero, false);

    m.properties.push(Property {
        name: format!("gray_code_{sig_name}"),
        kind: PropertyKind::Assert,
        cond: onehot0,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);

    match run_kinduction(
        &m,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )? {
        ProveOutcome::Proved { .. } => Ok(GrayStatus::Proved),
        ProveOutcome::Fail { step, trace, .. } => Ok(GrayStatus::Fail { step, trace }),
        ProveOutcome::AssumptionsUnsat { .. } => Ok(GrayStatus::Unknown),
        ProveOutcome::Unknown { .. } if cfg.use_pdr => match run_pdr(
            &m,
            &PdrConfig {
                smt_solver: cfg.solver.clone(),
                ..Default::default()
            },
        ) {
            Ok(ProveOutcome::Proved { .. }) => Ok(GrayStatus::Proved),
            Ok(ProveOutcome::Fail { step, trace, .. }) => Ok(GrayStatus::Fail { step, trace }),
            _ => Ok(GrayStatus::Unknown),
        },
        ProveOutcome::Unknown { .. } => Ok(GrayStatus::Unknown),
    }
}
