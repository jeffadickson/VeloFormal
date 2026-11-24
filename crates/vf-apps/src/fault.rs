use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{
    AsyncReset, CombAssign, ConstBits, ExprId, ExprKind, Ff, MemId, MemWrite, Module, Property,
    PropertyKind, SignalId, SignalKind,
};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultModel {
    /// Permanent Stuck-At-0 fault on the target bit.
    Sa0,
    /// Permanent Stuck-At-1 fault on the target bit.
    Sa1,
    /// Transient Single-Event Upset (single-cycle symbolic bit-flip at any cycle `t >= 0`).
    Seu,
}

impl FaultModel {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sa0" | "stuck0" | "stuck-at-0" => Ok(Self::Sa0),
            "sa1" | "stuck1" | "stuck-at-1" => Ok(Self::Sa1),
            "seu" | "transient" | "flip" => Ok(Self::Seu),
            other => bail!("unsupported fault model '{other}' (expected sa0, sa1, or seu)"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sa0 => "SA0",
            Self::Sa1 => "SA1",
            Self::Seu => "SEU",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FaultConfig {
    /// Target register/signal names to inject faults into (if empty, auto-selects all user FFs).
    pub targets: Vec<String>,
    /// Safety-critical output port names to observe (if empty, uses all module outputs except `alarm`).
    pub outputs: Vec<String>,
    /// Optional safety-mechanism error/alarm output signal (e.g., ECC/parity/lockstep/TMR err flag).
    pub alarm: Option<String>,
    /// Maximum cycles after output divergence within which `alarm` must fire (0 = same cycle or earlier).
    pub detect_latency: usize,
    /// Fault models to inject per target bit (defaults to `[Sa0, Sa1, Seu]`).
    pub models: Vec<FaultModel>,
    /// Initial settling cycles to skip before comparing safety outputs.
    pub skip_cycles: usize,
    /// Maximum total fault sites to analyze.
    pub max_faults: usize,
    /// Maximum k-induction depth.
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for FaultConfig {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            outputs: Vec::new(),
            alarm: None,
            detect_latency: 0,
            models: vec![FaultModel::Sa0, FaultModel::Sa1, FaultModel::Seu],
            skip_cycles: 0,
            max_faults: 64,
            depth: 12,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FaultStatus {
    /// Formally proved safe/masked: fault can NEVER corrupt any safety output under any input sequence.
    SafeMasked,
    /// Propagatable fault, but formally proved DETECTED by the safety mechanism (`alarm`) within `detect_latency`.
    Detected,
    /// Dangerous Single-Point Fault (SPF): concrete counterexample corrupting a safety output WITHOUT alarm!
    DangerousSpf { step: usize, trace: Vec<TraceFrame> },
    /// Solver inconclusive within configured depth/timeout.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct FaultFinding {
    pub signal: String,
    pub bit: u32,
    pub model: FaultModel,
    pub status: FaultStatus,
}

#[derive(Debug, Clone)]
pub struct FaultReport {
    pub total_faults: usize,
    pub safe_masked: usize,
    pub detected: usize,
    pub dangerous_spf: usize,
    pub unknown: usize,
    /// ISO 26262 Diagnostic Coverage (%) = `detected / (detected + dangerous_spf) * 100` (100% if no propagatable faults).
    pub diagnostic_coverage_pct: f64,
    /// ISO 26262 Single-Point Fault Metric (%) = `(safe_masked + detected) / total_faults * 100`.
    pub spfm_pct: f64,
    pub findings: Vec<FaultFinding>,
}

/// Questa Analyze Fault / ISO 26262 Formal Fault Analysis equivalent:
/// Builds a 2-copy fault-injection miter (`gold` vs `fault`) for every `(target, bit, model)`
/// and formally classifies each fault as **Safe/Masked**, **Detected** (by `--alarm`), or
/// **Dangerous Single-Point Fault (SPF)** with a concrete counterexample trace.
pub fn run_fault_analysis(base: &Module, cfg: &FaultConfig) -> Result<FaultReport> {
    let alarm_id = match &cfg.alarm {
        Some(name) => Some(
            base.find_signal(name)
                .ok_or_else(|| anyhow!("fault analysis: alarm signal '{name}' not found"))?,
        ),
        None => None,
    };

    // Determine safety-critical outputs.
    let output_ids: Vec<SignalId> = if !cfg.outputs.is_empty() {
        cfg.outputs
            .iter()
            .map(|n| {
                base.find_signal(n)
                    .ok_or_else(|| anyhow!("fault analysis: output signal '{n}' not found"))
            })
            .collect::<Result<_>>()?
    } else {
        base.signals
            .iter()
            .filter(|s| s.kind == SignalKind::Output && Some(s.id) != alarm_id)
            .map(|s| s.id)
            .collect()
    };
    if output_ids.is_empty() {
        bail!("fault analysis: no safety-critical output signals found (specify --output)");
    }

    // Determine target signals to inject faults into.
    let ff_set: HashSet<SignalId> = base.ffs.iter().map(|f| f.signal).collect();
    let comb_set: HashSet<SignalId> = base.comb.iter().map(|c| c.signal).collect();

    let target_ids: Vec<SignalId> = if !cfg.targets.is_empty() {
        cfg.targets
            .iter()
            .map(|n| {
                let sid = base
                    .find_signal(n)
                    .ok_or_else(|| anyhow!("fault analysis: target signal '{n}' not found"))?;
                if !ff_set.contains(&sid) && !comb_set.contains(&sid) {
                    bail!(
                        "fault analysis: target '{n}' is neither a register nor a driven internal/output wire"
                    );
                }
                Ok(sid)
            })
            .collect::<Result<_>>()?
    } else {
        base.ffs
            .iter()
            .map(|f| f.signal)
            .filter(|&sid| {
                let s = base.signal(sid);
                !s.name.starts_with('$') && Some(sid) != alarm_id
            })
            .collect()
    };
    if target_ids.is_empty() {
        bail!("fault analysis: no target registers/signals found for fault injection");
    }

    let models = if cfg.models.is_empty() {
        vec![FaultModel::Sa0, FaultModel::Sa1, FaultModel::Seu]
    } else {
        cfg.models.clone()
    };

    // Enumerate (signal, bit, model) fault sites up to `max_faults`.
    let mut sites: Vec<(SignalId, u32, FaultModel)> = Vec::new();
    'outer: for &sid in &target_ids {
        let w = base.signal(sid).width;
        for bit in 0..w {
            for &model in &models {
                if sites.len() >= cfg.max_faults {
                    break 'outer;
                }
                sites.push((sid, bit, model));
            }
        }
    }

    // Analyze fault sites (parallel across up to 4 worker threads for throughput).
    let max_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(1)
        .min(sites.len().max(1));

    let findings: Vec<FaultFinding> = if max_threads <= 1 || sites.len() <= 2 {
        sites
            .iter()
            .map(|&(sid, bit, model)| {
                analyze_single_fault(base, sid, bit, model, &output_ids, alarm_id, cfg)
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        let chunk_size = sites.len().div_ceil(max_threads);
        let mut results: Vec<Result<Vec<FaultFinding>>> = Vec::new();
        std::thread::scope(|s| {
            let handles: Vec<_> = sites
                .chunks(chunk_size)
                .map(|chunk| {
                    s.spawn(|| {
                        chunk
                            .iter()
                            .map(|&(sid, bit, model)| {
                                analyze_single_fault(
                                    base,
                                    sid,
                                    bit,
                                    model,
                                    &output_ids,
                                    alarm_id,
                                    cfg,
                                )
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            for h in handles {
                results.push(
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow!("fault worker panicked"))),
                );
            }
        });
        let mut flat = Vec::with_capacity(sites.len());
        for r in results {
            flat.extend(r?);
        }
        flat
    };

    let mut safe_masked = 0usize;
    let mut detected = 0usize;
    let mut dangerous_spf = 0usize;
    let mut unknown = 0usize;
    for f in &findings {
        match f.status {
            FaultStatus::SafeMasked => safe_masked += 1,
            FaultStatus::Detected => detected += 1,
            FaultStatus::DangerousSpf { .. } => dangerous_spf += 1,
            FaultStatus::Unknown => unknown += 1,
        }
    }
    let total_faults = findings.len();
    let propagatable = detected + dangerous_spf;
    let diagnostic_coverage_pct = if propagatable == 0 {
        100.0
    } else {
        (detected as f64 / propagatable as f64) * 100.0
    };
    let spfm_pct = if total_faults == 0 {
        100.0
    } else {
        ((safe_masked + detected) as f64 / total_faults as f64) * 100.0
    };

    Ok(FaultReport {
        total_faults,
        safe_masked,
        detected,
        dangerous_spf,
        unknown,
        diagnostic_coverage_pct,
        spfm_pct,
        findings,
    })
}

fn analyze_single_fault(
    base: &Module,
    target: SignalId,
    bit: u32,
    model: FaultModel,
    output_ids: &[SignalId],
    alarm_id: Option<SignalId>,
    cfg: &FaultConfig,
) -> Result<FaultFinding> {
    let sig_name = base.signal(target).name.clone();

    // 1. Check Safe/Masked: can ANY safety output ever diverge between `gold` and `fault`?
    let mut masked_miter = build_fault_miter(
        base,
        target,
        bit,
        model,
        output_ids,
        None,
        cfg.skip_cycles,
        0,
    )?;
    vf_ir::passes::coi_prune(&mut masked_miter);

    let masked_res = prove_miter(&masked_miter, cfg)?;
    let status = match masked_res {
        ProveOutcome::Proved { .. } => FaultStatus::SafeMasked,
        ProveOutcome::Fail { step, trace, .. } => {
            if let Some(alarm) = alarm_id {
                // 2. Output CAN diverge -> verify whether the safety mechanism (`alarm`) detects it!
                let mut det_miter = build_fault_miter(
                    base,
                    target,
                    bit,
                    model,
                    output_ids,
                    Some(alarm),
                    cfg.skip_cycles,
                    cfg.detect_latency,
                )?;
                vf_ir::passes::coi_prune(&mut det_miter);
                match prove_miter(&det_miter, cfg)? {
                    ProveOutcome::Proved { .. } => FaultStatus::Detected,
                    ProveOutcome::Fail {
                        step: dstep,
                        trace: dtrace,
                        ..
                    } => FaultStatus::DangerousSpf {
                        step: dstep,
                        trace: dtrace,
                    },
                    _ => FaultStatus::Unknown,
                }
            } else {
                FaultStatus::DangerousSpf { step, trace }
            }
        }
        _ => FaultStatus::Unknown,
    };

    Ok(FaultFinding {
        signal: sig_name,
        bit,
        model,
        status,
    })
}

fn prove_miter(miter: &Module, cfg: &FaultConfig) -> Result<ProveOutcome> {
    let k = run_kinduction(
        miter,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )?;
    match k {
        ProveOutcome::Unknown { .. } if cfg.use_pdr => Ok(run_pdr(
            miter,
            &PdrConfig {
                smt_solver: cfg.solver.clone(),
                ..Default::default()
            },
        )
        .unwrap_or(k)),
        other => Ok(other),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_fault_miter(
    base: &Module,
    target: SignalId,
    bit: u32,
    model: FaultModel,
    output_ids: &[SignalId],
    alarm_id: Option<SignalId>,
    skip_cycles: usize,
    detect_latency: usize,
) -> Result<Module> {
    let mut miter = Module::new(format!(
        "fault_{}_{}_b{}_{}",
        base.name,
        base.signal(target).name,
        bit,
        model.as_str()
    ));

    // Share all primary inputs between `gold` and `fault` copies.
    let mut shared_inputs: HashMap<SignalId, SignalId> = HashMap::new();
    for s in &base.signals {
        if s.kind == SignalKind::Input {
            let id = miter.add_signal(&s.name, s.width, s.signed, SignalKind::Input);
            miter.signal_mut(id).is_clock = s.is_clock;
            shared_inputs.insert(s.id, id);
        }
    }

    let gold_map = embed_fault_copy(&mut miter, base, "gold", &shared_inputs, None);
    let fault_map = embed_fault_copy(
        &mut miter,
        base,
        "fault",
        &shared_inputs,
        Some((target, bit, model)),
    );

    // Optional settling mask: 0 for `0..skip_cycles`, then 1 forever.
    let mut valid_en = miter.c_bool(true);
    for i in 0..skip_cycles {
        let s = miter.add_signal(format!("$fault_valid_{i}"), 1, false, SignalKind::Reg);
        miter.signal_mut(s).init = Some(ConstBits::zero(1));
        miter.ffs.push(Ff {
            signal: s,
            next: valid_en,
            init: Some(ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });
        valid_en = miter.sig_expr(s);
    }

    // Compute combined safety output mismatch: `any_mismatch = OR_i (gold.out_i != fault.out_i)`.
    let mut any_mismatch = miter.c_bool(false);
    for &oid in output_ids {
        let g = miter.sig_expr(gold_map[&oid]);
        let f = miter.sig_expr(fault_map[&oid]);
        let neq = miter.intern(1, ExprKind::Ne(g, f));
        any_mismatch = miter.logic_or(any_mismatch, neq);
    }

    if let Some(aid) = alarm_id {
        // Safety Mechanism Detection Check:
        // Whenever `any_mismatch` occurs (after `skip_cycles`), `fault.<alarm>` must have asserted
        // at or before `t + detect_latency`.
        let alarm_raw = miter.sig_expr(fault_map[&aid]);
        let alarm_b = miter.to_bool(alarm_raw);
        let alarm_seen_reg = miter.add_signal("$fault_alarm_seen", 1, false, SignalKind::Reg);
        miter.signal_mut(alarm_seen_reg).init = Some(ConstBits::zero(1));
        let alarm_seen_e = miter.sig_expr(alarm_seen_reg);
        let alarm_or_seen = miter.logic_or(alarm_seen_e, alarm_b);
        miter.ffs.push(Ff {
            signal: alarm_seen_reg,
            next: alarm_or_seen,
            init: Some(ConstBits::zero(1)),
            clock: None,
            async_reset: None,
        });

        let mut trig = miter.logic_and(valid_en, any_mismatch);
        for d in 0..detect_latency {
            let ds = miter.add_signal(format!("$fault_mm_d_{d}"), 1, false, SignalKind::Reg);
            miter.signal_mut(ds).init = Some(ConstBits::zero(1));
            miter.ffs.push(Ff {
                signal: ds,
                next: trig,
                init: Some(ConstBits::zero(1)),
                clock: None,
                async_reset: None,
            });
            trig = miter.sig_expr(ds);
        }

        // Assert: `trig => alarm_or_seen` (en = trig, cond = alarm_or_seen).
        miter.properties.push(Property {
            name: format!(
                "fault_detected_{}_b{}_{}",
                base.signal(target).name,
                bit,
                model.as_str()
            ),
            kind: PropertyKind::Assert,
            cond: alarm_or_seen,
            en: trig,
            source: None,
        });
    } else {
        // Masked / Safe Check: assert `!any_mismatch` whenever `valid_en` holds.
        let no_mismatch = miter.logic_not(any_mismatch);
        miter.properties.push(Property {
            name: format!(
                "fault_safe_{}_b{}_{}",
                base.signal(target).name,
                bit,
                model.as_str()
            ),
            kind: PropertyKind::Assert,
            cond: no_mismatch,
            en: valid_en,
            source: None,
        });
    }

    miter.topo_sort_comb().map_err(|e| anyhow!(e))?;
    Ok(miter)
}

fn bit_mask_const(width: u32, bit: u32, invert: bool) -> ConstBits {
    let mut s = String::with_capacity(width as usize);
    for pos in (0..width).rev() {
        let is_target = pos == bit;
        let ch = if invert {
            if is_target {
                '0'
            } else {
                '1'
            }
        } else if is_target {
            '1'
        } else {
            '0'
        };
        s.push(ch);
    }
    ConstBits::new(width, s)
}

fn embed_fault_copy(
    dst: &mut Module,
    src: &Module,
    prefix: &str,
    shared_inputs: &HashMap<SignalId, SignalId>,
    fault: Option<(SignalId, u32, FaultModel)>,
) -> HashMap<SignalId, SignalId> {
    let fault_target = fault.map(|(sid, _, _)| sid);
    let mut sig_map: HashMap<SignalId, SignalId> = HashMap::new();
    let mut fault_raw_sig: Option<SignalId> = None;

    for s in &src.signals {
        let id = if let Some(&shared) = shared_inputs.get(&s.id) {
            shared
        } else if Some(s.id) == fault_target {
            // Create a raw signal holding the unfaulted driver (FF or comb), and expose
            // `prefix.<name>` as a Wire carrying the fault-injected value!
            let raw_kind = if s.kind == SignalKind::Output {
                SignalKind::Wire
            } else {
                s.kind
            };
            let raw_id = dst.add_signal(format!("{prefix}.{}$raw", s.name), s.width, s.signed, raw_kind);
            dst.signal_mut(raw_id).init = s.init.clone();
            fault_raw_sig = Some(raw_id);

            dst.add_signal(format!("{prefix}.{}", s.name), s.width, s.signed, SignalKind::Wire)
        } else {
            let kind = if s.kind == SignalKind::Output {
                SignalKind::Wire
            } else {
                s.kind
            };
            let nid = dst.add_signal(format!("{prefix}.{}", s.name), s.width, s.signed, kind);
            dst.signal_mut(nid).init = s.init.clone();
            nid
        };
        sig_map.insert(s.id, id);
    }

    let mut mem_map: HashMap<MemId, MemId> = HashMap::new();
    for mem in &src.memories {
        let nid = dst.add_memory(
            format!("{prefix}.{}", mem.name),
            mem.addr_width,
            mem.data_width,
            mem.depth,
        );
        dst.memory_mut(nid).init = mem.init.clone();
        mem_map.insert(mem.id, nid);
    }

    let mut expr_map: Vec<ExprId> = Vec::with_capacity(src.exprs.len());
    for node in &src.exprs {
        let kind = match &node.kind {
            ExprKind::Const(c) => ExprKind::Const(c.clone()),
            ExprKind::Signal(s) => ExprKind::Signal(sig_map[s]),
            ExprKind::Not(a) => ExprKind::Not(expr_map[a.0]),
            ExprKind::Neg(a) => ExprKind::Neg(expr_map[a.0]),
            ExprKind::RedAnd(a) => ExprKind::RedAnd(expr_map[a.0]),
            ExprKind::RedOr(a) => ExprKind::RedOr(expr_map[a.0]),
            ExprKind::RedXor(a) => ExprKind::RedXor(expr_map[a.0]),
            ExprKind::Add(a, b) => ExprKind::Add(expr_map[a.0], expr_map[b.0]),
            ExprKind::Sub(a, b) => ExprKind::Sub(expr_map[a.0], expr_map[b.0]),
            ExprKind::Mul(a, b) => ExprKind::Mul(expr_map[a.0], expr_map[b.0]),
            ExprKind::UDiv(a, b) => ExprKind::UDiv(expr_map[a.0], expr_map[b.0]),
            ExprKind::SDiv(a, b) => ExprKind::SDiv(expr_map[a.0], expr_map[b.0]),
            ExprKind::UMod(a, b) => ExprKind::UMod(expr_map[a.0], expr_map[b.0]),
            ExprKind::SMod(a, b) => ExprKind::SMod(expr_map[a.0], expr_map[b.0]),
            ExprKind::And(a, b) => ExprKind::And(expr_map[a.0], expr_map[b.0]),
            ExprKind::Or(a, b) => ExprKind::Or(expr_map[a.0], expr_map[b.0]),
            ExprKind::Xor(a, b) => ExprKind::Xor(expr_map[a.0], expr_map[b.0]),
            ExprKind::Shl(a, b) => ExprKind::Shl(expr_map[a.0], expr_map[b.0]),
            ExprKind::Lshr(a, b) => ExprKind::Lshr(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ashr(a, b) => ExprKind::Ashr(expr_map[a.0], expr_map[b.0]),
            ExprKind::Eq(a, b) => ExprKind::Eq(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ne(a, b) => ExprKind::Ne(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ult(a, b) => ExprKind::Ult(expr_map[a.0], expr_map[b.0]),
            ExprKind::Ule(a, b) => ExprKind::Ule(expr_map[a.0], expr_map[b.0]),
            ExprKind::Slt(a, b) => ExprKind::Slt(expr_map[a.0], expr_map[b.0]),
            ExprKind::Sle(a, b) => ExprKind::Sle(expr_map[a.0], expr_map[b.0]),
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => ExprKind::Mux {
                cond: expr_map[cond.0],
                then_val: expr_map[then_val.0],
                else_val: expr_map[else_val.0],
            },
            ExprKind::Concat(parts) => {
                ExprKind::Concat(parts.iter().map(|p| expr_map[p.0]).collect())
            }
            ExprKind::Slice { expr, hi, lo } => ExprKind::Slice {
                expr: expr_map[expr.0],
                hi: *hi,
                lo: *lo,
            },
            ExprKind::Zext { expr, width } => ExprKind::Zext {
                expr: expr_map[expr.0],
                width: *width,
            },
            ExprKind::Sext { expr, width } => ExprKind::Sext {
                expr: expr_map[expr.0],
                width: *width,
            },
            ExprKind::MemRead { mem, addr } => ExprKind::MemRead {
                mem: mem_map[mem],
                addr: expr_map[addr.0],
            },
        };
        expr_map.push(dst.intern(node.width, kind));
    }

    for ca in &src.comb {
        let dst_sig = if Some(ca.signal) == fault_target {
            fault_raw_sig.unwrap()
        } else {
            sig_map[&ca.signal]
        };
        dst.comb.push(CombAssign {
            signal: dst_sig,
            value: expr_map[ca.value.0],
        });
    }
    for ff in &src.ffs {
        let dst_sig = if Some(ff.signal) == fault_target {
            fault_raw_sig.unwrap()
        } else {
            sig_map[&ff.signal]
        };
        dst.ffs.push(Ff {
            signal: dst_sig,
            next: expr_map[ff.next.0],
            init: ff.init.clone(),
            clock: ff.clock.map(|c| vf_ir::ClockSpec {
                signal: sig_map[&c.signal],
                edge: c.edge,
            }),
            async_reset: ff.async_reset.as_ref().map(|r| AsyncReset {
                cond: expr_map[r.cond.0],
                reset_val: expr_map[r.reset_val.0],
            }),
        });
    }
    for mw in &src.mem_writes {
        dst.mem_writes.push(MemWrite {
            mem: mem_map[&mw.mem],
            addr: expr_map[mw.addr.0],
            data: expr_map[mw.data.0],
            enable: expr_map[mw.enable.0],
            clock: mw.clock.map(|c| vf_ir::ClockSpec {
                signal: sig_map[&c.signal],
                edge: c.edge,
            }),
        });
    }
    for p in &src.properties {
        if p.kind == PropertyKind::Assume {
            dst.properties.push(Property {
                name: format!("{prefix}.{}", p.name),
                kind: PropertyKind::Assume,
                cond: expr_map[p.cond.0],
                en: expr_map[p.en.0],
                source: p.source.clone(),
            });
        }
    }

    // Wire the fault injection multiplexer/bitmask from `raw_sig` -> `sig_map[target]`.
    if let (Some((tid, bit, model)), Some(raw_sig)) = (fault, fault_raw_sig) {
        let w = src.signal(tid).width;
        let raw_e = dst.sig_expr(raw_sig);
        let injected = match model {
            FaultModel::Sa0 => {
                let mask = dst.c_bits(bit_mask_const(w, bit, true));
                dst.intern(w, ExprKind::And(raw_e, mask))
            }
            FaultModel::Sa1 => {
                let mask = dst.c_bits(bit_mask_const(w, bit, false));
                dst.intern(w, ExprKind::Or(raw_e, mask))
            }
            FaultModel::Seu => {
                // Single-cycle transient bit flip at an unconstrained cycle `t >= 0` (at most once).
                let trig = dst.add_signal("$fault_seu_trig", 1, false, SignalKind::AnySeq);
                let fired = dst.add_signal("$fault_seu_fired", 1, false, SignalKind::Reg);
                dst.signal_mut(fired).init = Some(ConstBits::zero(1));

                let trig_e = dst.sig_expr(trig);
                let fired_e = dst.sig_expr(fired);
                let not_fired = dst.logic_not(fired_e);
                let pulse = dst.logic_and(trig_e, not_fired);
                let next_fired = dst.logic_or(fired_e, trig_e);
                dst.ffs.push(Ff {
                    signal: fired,
                    next: next_fired,
                    init: Some(ConstBits::zero(1)),
                    clock: None,
                    async_reset: None,
                });

                let bit_mask = dst.c_bits(bit_mask_const(w, bit, false));
                let flipped = dst.intern(w, ExprKind::Xor(raw_e, bit_mask));
                dst.mux(pulse, flipped, raw_e)
            }
        };
        dst.comb.push(CombAssign {
            signal: sig_map[&tid],
            value: injected,
        });
    }

    sig_map
}
