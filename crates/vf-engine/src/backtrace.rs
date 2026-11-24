use std::collections::{HashMap, HashSet};
use vf_ir::{Module, PropertyKind, SignalKind};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone)]
pub struct BacktraceEvent {
    pub cycle: usize,
    pub signal: String,
    pub role: &'static str,
    pub prev_val: Option<String>,
    pub new_val: String,
}

#[derive(Debug, Clone)]
pub struct CexBacktrace {
    pub property: String,
    pub failing_cycle: usize,
    pub coi_signals: Vec<String>,
    pub events: Vec<BacktraceEvent>,
}

/// Questa PropCheck Root-Cause Counterexample Backtrace:
/// Computes the Cone-of-Influence (COI) of the failing assertion and extracts the minimal causal
/// chain of COI signal transitions leading up to the failure cycle.
pub fn compute_cex_backtrace(
    m: &Module,
    failed_prop: Option<&str>,
    trace: &[TraceFrame],
) -> CexBacktrace {
    let failing_cycle = trace.len().saturating_sub(1);
    let prop_name = failed_prop.unwrap_or("assertion").to_string();

    // Build a COI-pruned view focused specifically on the failing property (if known).
    let mut sub = m.clone();
    if let Some(target_name) = failed_prop {
        sub.properties
            .retain(|p| p.kind == PropertyKind::Assume || p.name == target_name);
    }
    vf_ir::passes::coi_prune(&mut sub);

    let ff_set: HashSet<_> = sub.ffs.iter().map(|f| f.signal).collect();
    let comb_set: HashSet<_> = sub.comb.iter().map(|c| c.signal).collect();

    // Collect all signals referenced by any expression in `sub` (inputs + driven FFs/wires).
    let mut coi_names: HashSet<String> = HashSet::new();
    let mut roles: HashMap<String, &'static str> = HashMap::new();
    for s in &sub.signals {
        if s.name.starts_with('$') || s.is_clock {
            continue;
        }
        let in_coi = s.kind == SignalKind::Input
            || s.kind == SignalKind::AnySeq
            || s.kind == SignalKind::AnyConst
            || ff_set.contains(&s.id)
            || comb_set.contains(&s.id);
        if in_coi {
            let role = if ff_set.contains(&s.id) {
                "reg"
            } else {
                match s.kind {
                    SignalKind::Input | SignalKind::AnySeq | SignalKind::AnyConst => "input",
                    SignalKind::Output => "output",
                    SignalKind::Reg => "reg",
                    SignalKind::Wire => "wire",
                }
            };
            coi_names.insert(s.name.clone());
            roles.insert(s.name.clone(), role);
        }
    }

    // Fallback if trace comes from a miter/transformed module whose names differ slightly.
    if coi_names.is_empty() && !trace.is_empty() {
        for (name, _) in &trace[0].values {
            if !name.starts_with('$') {
                coi_names.insert(name.clone());
                roles.insert(name.clone(), "signal");
            }
        }
    }

    let mut coi_list: Vec<String> = coi_names.iter().cloned().collect();
    coi_list.sort();

    // Walk cycles 0..trace.len(), recording COI signal transitions and failing-cycle state.
    let mut events = Vec::new();
    let mut prev_map: HashMap<String, String> = HashMap::new();

    for (t, frame) in trace.iter().enumerate() {
        for (name, bits) in &frame.values {
            if !coi_names.contains(name) {
                continue;
            }
            let val_str = format_val(bits);
            let role = roles.get(name).copied().unwrap_or("signal");
            match prev_map.get(name) {
                None => {
                    // Record cycle-0 non-zero inputs/regs or all failing-cycle values when failing_cycle == 0.
                    if t == failing_cycle || (!bits.is_zero() && (role == "input" || role == "reg"))
                    {
                        events.push(BacktraceEvent {
                            cycle: t,
                            signal: name.clone(),
                            role,
                            prev_val: None,
                            new_val: val_str.clone(),
                        });
                    }
                }
                Some(prev) if prev != &val_str => {
                    events.push(BacktraceEvent {
                        cycle: t,
                        signal: name.clone(),
                        role,
                        prev_val: Some(prev.clone()),
                        new_val: val_str.clone(),
                    });
                }
                _ => {}
            }
            prev_map.insert(name.clone(), val_str);
        }
    }

    // Keep output compact (prioritize events closest to failure if trace is very large).
    if events.len() > 24 {
        let keep_from = events.len() - 24;
        events = events.split_off(keep_from);
    }

    CexBacktrace {
        property: prop_name,
        failing_cycle,
        coi_signals: coi_list,
        events,
    }
}

fn format_val(bits: &vf_ir::ConstBits) -> String {
    if bits.width <= 4 {
        format!("{}'b{}", bits.width, bits.bits)
    } else if let Some(v) = bits.to_u64() {
        format!("0x{v:x}")
    } else {
        format!("{}'b{}", bits.width, bits.bits)
    }
}

impl CexBacktrace {
    pub fn format_text(&self) -> String {
        if self.events.is_empty() {
            return String::new();
        }
        let mut out = format!(
            "  Root-Cause Backtrace ({} failed at cycle {}, {} COI signals):\n",
            self.property,
            self.failing_cycle,
            self.coi_signals.len()
        );
        for ev in &self.events {
            if let Some(prev) = &ev.prev_val {
                out.push_str(&format!(
                    "    [t={}] ({}) {} : {} -> {}\n",
                    ev.cycle, ev.role, ev.signal, prev, ev.new_val
                ));
            } else {
                out.push_str(&format!(
                    "    [t={}] ({}) {} = {}\n",
                    ev.cycle, ev.role, ev.signal, ev.new_val
                ));
            }
        }
        out
    }
}
