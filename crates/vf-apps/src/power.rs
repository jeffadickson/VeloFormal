use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{ConstBits, ExprId, ExprKind, Ff, Module, Property, PropertyKind, SignalId, SignalKind};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClampValue {
    Zero,
    One,
    Latch,
}

impl ClampValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::One => "1",
            Self::Latch => "latch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoSense {
    High,
    Low,
}

#[derive(Debug, Clone)]
pub struct PowerDomain {
    pub name: String,
    pub elements: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IsolationRule {
    pub name: String,
    pub domain: String,
    pub clamp: ClampValue,
    pub elements: Vec<String>,
    pub iso_signal: Option<String>,
    pub iso_sense: IsoSense,
}

#[derive(Debug, Clone)]
pub struct RetentionRule {
    pub name: String,
    pub domain: String,
    pub elements: Vec<String>,
    pub save_signal: Option<(String, IsoSense)>,
    pub restore_signal: Option<(String, IsoSense)>,
}

#[derive(Debug, Clone, Default)]
pub struct UpfSpec {
    pub domains: Vec<PowerDomain>,
    pub isolations: Vec<IsolationRule>,
    pub retentions: Vec<RetentionRule>,
}

#[derive(Debug, Clone)]
pub struct PowerCheckConfig {
    /// Optional domain power-off signal mappings (`domain_name -> signal_name`) to prove
    /// isolation is always asserted whenever the power domain is powered off.
    pub pd_off: Vec<(String, String)>,
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

impl Default for PowerCheckConfig {
    fn default() -> Self {
        Self {
            pd_off: Vec::new(),
            depth: 15,
            solver: "z3".to_string(),
            use_pdr: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PowerStatus {
    Pass,
    Fail { step: usize, trace: Vec<TraceFrame> },
    StructuralViolation(String),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PowerFinding {
    pub domain: String,
    pub rule: String,
    pub signal: String,
    pub check: String,
    pub status: PowerStatus,
}

/// Parse an IEEE 1801 UPF (`.upf`) file.
pub fn parse_upf_file(path: &Path) -> Result<UpfSpec> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read UPF file {}", path.display()))?;
    parse_upf_str(&text)
}

/// Parse IEEE 1801 UPF subset (`create_power_domain`, `set_isolation`, `set_isolation_control`).
pub fn parse_upf_str(text: &str) -> Result<UpfSpec> {
    let mut spec = UpfSpec::default();
    let logical_lines = join_tcl_continuations(text);

    for raw_line in logical_lines {
        let line = strip_tcl_comment(&raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        let tokens = tokenize_tcl(&line);
        if tokens.is_empty() {
            continue;
        }
        match tokens[0].as_str() {
            "create_power_domain" => {
                if tokens.len() < 2 {
                    bail!("UPF `create_power_domain` missing domain name");
                }
                let name = tokens[1].clone();
                let mut elements = Vec::new();
                let mut i = 2;
                while i < tokens.len() {
                    if tokens[i] == "-elements" && i + 1 < tokens.len() {
                        elements.extend(split_list_tokens(&tokens[i + 1]));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                spec.domains.push(PowerDomain { name, elements });
            }
            "set_isolation" => {
                if tokens.len() < 2 {
                    bail!("UPF `set_isolation` missing strategy name");
                }
                let name = tokens[1].clone();
                let mut domain = String::new();
                let mut clamp = ClampValue::Zero;
                let mut elements = Vec::new();
                let mut iso_signal = None;
                let mut iso_sense = IsoSense::High;

                let mut i = 2;
                while i < tokens.len() {
                    match tokens[i].as_str() {
                        "-domain" if i + 1 < tokens.len() => {
                            domain = tokens[i + 1].clone();
                            i += 2;
                        }
                        "-clamp_value" if i + 1 < tokens.len() => {
                            clamp = match tokens[i + 1].to_ascii_lowercase().as_str() {
                                "0" | "zero" => ClampValue::Zero,
                                "1" | "one" => ClampValue::One,
                                "latch" | "hold" => ClampValue::Latch,
                                other => bail!("UPF unsupported -clamp_value '{other}'"),
                            };
                            i += 2;
                        }
                        "-elements" if i + 1 < tokens.len() => {
                            elements.extend(split_list_tokens(&tokens[i + 1]));
                            i += 2;
                        }
                        "-isolation_signal" if i + 1 < tokens.len() => {
                            iso_signal = Some(tokens[i + 1].clone());
                            i += 2;
                        }
                        "-isolation_sense" if i + 1 < tokens.len() => {
                            iso_sense = match tokens[i + 1].to_ascii_lowercase().as_str() {
                                "low" | "0" => IsoSense::Low,
                                _ => IsoSense::High,
                            };
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
                if domain.is_empty() {
                    bail!("UPF `set_isolation {name}` missing `-domain`");
                }
                spec.isolations.push(IsolationRule {
                    name,
                    domain,
                    clamp,
                    elements,
                    iso_signal,
                    iso_sense,
                });
            }
            "set_isolation_control" => {
                if tokens.len() < 2 {
                    bail!("UPF `set_isolation_control` missing strategy name");
                }
                let name = &tokens[1];
                let mut domain = None;
                let mut iso_signal = None;
                let mut iso_sense = None;

                let mut i = 2;
                while i < tokens.len() {
                    match tokens[i].as_str() {
                        "-domain" if i + 1 < tokens.len() => {
                            domain = Some(tokens[i + 1].clone());
                            i += 2;
                        }
                        "-isolation_signal" if i + 1 < tokens.len() => {
                            iso_signal = Some(tokens[i + 1].clone());
                            i += 2;
                        }
                        "-isolation_sense" if i + 1 < tokens.len() => {
                            iso_sense = Some(match tokens[i + 1].to_ascii_lowercase().as_str() {
                                "low" | "0" => IsoSense::Low,
                                _ => IsoSense::High,
                            });
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }

                let rule = spec
                    .isolations
                    .iter_mut()
                    .find(|r| &r.name == name && domain.as_ref().is_none_or(|d| d == &r.domain))
                    .ok_or_else(|| {
                        anyhow!("UPF `set_isolation_control` references unknown strategy '{name}'")
                    })?;
                if let Some(sig) = iso_signal {
                    rule.iso_signal = Some(sig);
                }
                if let Some(sense) = iso_sense {
                    rule.iso_sense = sense;
                }
            }
            "set_retention" => {
                if tokens.len() < 2 {
                    bail!("UPF `set_retention` missing strategy name");
                }
                let name = tokens[1].clone();
                let mut domain = String::new();
                let mut elements = Vec::new();
                let mut save_signal = None;
                let mut restore_signal = None;
                let mut i = 2;
                while i < tokens.len() {
                    match tokens[i].as_str() {
                        "-domain" if i + 1 < tokens.len() => {
                            domain = tokens[i + 1].clone();
                            i += 2;
                        }
                        "-elements" if i + 1 < tokens.len() => {
                            elements.extend(split_list_tokens(&tokens[i + 1]));
                            i += 2;
                        }
                        "-save_signal" if i + 1 < tokens.len() => {
                            save_signal = parse_sense_pair(&tokens[i + 1]);
                            i += 2;
                        }
                        "-restore_signal" if i + 1 < tokens.len() => {
                            restore_signal = parse_sense_pair(&tokens[i + 1]);
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
                if domain.is_empty() {
                    bail!("UPF `set_retention {name}` missing `-domain`");
                }
                spec.retentions.push(RetentionRule {
                    name,
                    domain,
                    elements,
                    save_signal,
                    restore_signal,
                });
            }
            "set_retention_control" => {
                if tokens.len() < 2 {
                    bail!("UPF `set_retention_control` missing strategy name");
                }
                let name = &tokens[1];
                let mut domain = None;
                let mut save_signal = None;
                let mut restore_signal = None;
                let mut i = 2;
                while i < tokens.len() {
                    match tokens[i].as_str() {
                        "-domain" if i + 1 < tokens.len() => {
                            domain = Some(tokens[i + 1].clone());
                            i += 2;
                        }
                        "-save_signal" if i + 1 < tokens.len() => {
                            save_signal = parse_sense_pair(&tokens[i + 1]);
                            i += 2;
                        }
                        "-restore_signal" if i + 1 < tokens.len() => {
                            restore_signal = parse_sense_pair(&tokens[i + 1]);
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
                let rule = spec
                    .retentions
                    .iter_mut()
                    .find(|r| &r.name == name && domain.as_ref().is_none_or(|d| d == &r.domain))
                    .ok_or_else(|| {
                        anyhow!("UPF `set_retention_control` references unknown strategy '{name}'")
                    })?;
                if save_signal.is_some() {
                    rule.save_signal = save_signal;
                }
                if restore_signal.is_some() {
                    rule.restore_signal = restore_signal;
                }
            }
            _ => {
                // Ignore informational UPF supply/PST statements cleanly.
            }
        }
    }

    if spec.isolations.is_empty() && spec.domains.is_empty() && spec.retentions.is_empty() {
        bail!("UPF specification contained no power domains, isolation, or retention strategies");
    }
    Ok(spec)
}

fn parse_sense_pair(tok: &str) -> Option<(String, IsoSense)> {
    let parts = split_list_tokens(tok);
    let sig = parts.first()?.clone();
    let sense = match parts
        .get(1)
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("low" | "0" | "negedge") => IsoSense::Low,
        _ => IsoSense::High,
    };
    Some((sig, sense))
}

/// Questa Low-Power / UPF Power-Domain Crossing Verifier:
/// 1. Proves that every isolated signal clamps to its IEEE 1801 UPF `clamp_value` (`0`, `1`, or `latch`)
///    whenever the isolation control signal is active.
/// 2. Verifies structural isolation completeness (flags un-isolated signals in switchable domains).
/// 3. When `--pd-off <domain>=<sig>` is given, proves isolation is active whenever the domain is powered off.
pub fn run_powercheck(
    base: &Module,
    upf: &UpfSpec,
    cfg: &PowerCheckConfig,
) -> Result<Vec<PowerFinding>> {
    let mut findings = Vec::new();
    let domain_map: HashMap<&str, &PowerDomain> = upf
        .domains
        .iter()
        .map(|d| (d.name.as_str(), d))
        .collect();

    let mut isolated_by_domain: HashMap<&str, HashSet<String>> = HashMap::new();

    // 1. Formal clamp verification for each UPF isolation rule.
    for rule in &upf.isolations {
        let Some(iso_sig_name) = &rule.iso_signal else {
            findings.push(PowerFinding {
                domain: rule.domain.clone(),
                rule: rule.name.clone(),
                signal: "*".into(),
                check: "iso_control".into(),
                status: PowerStatus::StructuralViolation(format!(
                    "isolation strategy '{}' has no `-isolation_signal` control",
                    rule.name
                )),
            });
            continue;
        };
        let iso_sid = base.find_signal(iso_sig_name).ok_or_else(|| {
            anyhow!("UPF isolation signal '{iso_sig_name}' not found in RTL module '{}'", base.name)
        })?;

        let targets: Vec<String> = if !rule.elements.is_empty() {
            rule.elements.clone()
        } else if let Some(dom) = domain_map.get(rule.domain.as_str()) {
            dom.elements.clone()
        } else {
            Vec::new()
        };

        for tname in &targets {
            isolated_by_domain
                .entry(rule.domain.as_str())
                .or_default()
                .insert(tname.clone());

            let tsid = base.find_signal(tname).ok_or_else(|| {
                anyhow!("UPF isolated signal '{tname}' not found in RTL module '{}'", base.name)
            })?;

            let status = verify_clamp(base, iso_sid, rule.iso_sense, tsid, rule.clamp, cfg)?;
            findings.push(PowerFinding {
                domain: rule.domain.clone(),
                rule: rule.name.clone(),
                signal: tname.clone(),
                check: format!("clamp_{}", rule.clamp.as_str()),
                status,
            });
        }

        // 2. If `--pd-off <domain>=<sig>` is configured for this domain, prove `pd_off => iso_active`.
        for (dom_name, pd_off_sig) in &cfg.pd_off {
            if dom_name == &rule.domain {
                let off_sid = base.find_signal(pd_off_sig).ok_or_else(|| {
                    anyhow!("power-off signal '{pd_off_sig}' not found in RTL")
                })?;
                let status = verify_pd_off_isolated(base, off_sid, iso_sid, rule.iso_sense, cfg)?;
                findings.push(PowerFinding {
                    domain: rule.domain.clone(),
                    rule: rule.name.clone(),
                    signal: format!("{pd_off_sig} -> {iso_sig_name}"),
                    check: "pd_off_iso_seq".into(),
                    status,
                });
            }
        }
    }

    // 3. Structural un-isolated crossing check for switchable domains.
    for dom in &upf.domains {
        let Some(covered) = isolated_by_domain.get(dom.name.as_str()) else {
            continue;
        };
        let dom_elem_set: HashSet<SignalId> = dom
            .elements
            .iter()
            .filter_map(|e| base.find_signal(e))
            .collect();

        for elem in &dom.elements {
            if covered.contains(elem) {
                continue;
            }
            let Some(esid) = base.find_signal(elem) else {
                continue;
            };
            // Flag if this un-isolated domain signal is a module output OR feeds logic outside the domain!
            let is_output = base.signal(esid).kind == SignalKind::Output;
            let feeds_outside = signal_feeds_outside(base, esid, &dom_elem_set);
            if is_output || feeds_outside {
                findings.push(PowerFinding {
                    domain: dom.name.clone(),
                    rule: "unisolated".into(),
                    signal: elem.clone(),
                    check: "unisolated_crossing".into(),
                    status: PowerStatus::StructuralViolation(format!(
                        "signal '{elem}' in switchable domain '{}' crosses out without an isolation rule",
                        dom.name
                    )),
                });
            }
        }
    }

    // 4. Formal retention save/restore verification for IEEE 1801 `set_retention` strategies.
    for ret in &upf.retentions {
        let (Some((save_name, save_sense)), Some((rest_name, rest_sense))) =
            (&ret.save_signal, &ret.restore_signal)
        else {
            findings.push(PowerFinding {
                domain: ret.domain.clone(),
                rule: ret.name.clone(),
                signal: "*".into(),
                check: "retention_control".into(),
                status: PowerStatus::StructuralViolation(format!(
                    "retention strategy '{}' missing `-save_signal` or `-restore_signal`",
                    ret.name
                )),
            });
            continue;
        };
        let save_sid = base.find_signal(save_name).ok_or_else(|| {
            anyhow!(
                "UPF retention save signal '{save_name}' not found in RTL module '{}'",
                base.name
            )
        })?;
        let rest_sid = base.find_signal(rest_name).ok_or_else(|| {
            anyhow!(
                "UPF retention restore signal '{rest_name}' not found in RTL module '{}'",
                base.name
            )
        })?;

        let targets: Vec<String> = if !ret.elements.is_empty() {
            ret.elements.clone()
        } else if let Some(dom) = domain_map.get(ret.domain.as_str()) {
            dom.elements.clone()
        } else {
            Vec::new()
        };

        for tname in &targets {
            let tsid = base.find_signal(tname).ok_or_else(|| {
                anyhow!(
                    "UPF retention element '{tname}' not found in RTL module '{}'",
                    base.name
                )
            })?;
            let status = verify_retention(
                base,
                save_sid,
                *save_sense,
                rest_sid,
                *rest_sense,
                tsid,
                cfg,
            )?;
            findings.push(PowerFinding {
                domain: ret.domain.clone(),
                rule: ret.name.clone(),
                signal: tname.clone(),
                check: "retention_save_restore".into(),
                status,
            });
        }
    }

    Ok(findings)
}

fn verify_retention(
    base: &Module,
    save_sid: SignalId,
    save_sense: IsoSense,
    rest_sid: SignalId,
    rest_sense: IsoSense,
    target_sid: SignalId,
    cfg: &PowerCheckConfig,
) -> Result<PowerStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let save_raw = m.sig_expr(save_sid);
    let save_b = m.to_bool(save_raw);
    let save_act = match save_sense {
        IsoSense::High => save_b,
        IsoSense::Low => m.logic_not(save_b),
    };

    let rest_raw = m.sig_expr(rest_sid);
    let rest_b = m.to_bool(rest_raw);
    let rest_act = match rest_sense {
        IsoSense::High => rest_b,
        IsoSense::Low => m.logic_not(rest_b),
    };

    // Detect optional hardware reset so reset cleanly invalidates saved retention state.
    let rst_act = ["rst", "rst_n", "reset", "reset_n", "presetn"]
        .iter()
        .find_map(|cand| {
            m.find_signal(cand).and_then(|sid| {
                if m.signal(sid).kind == SignalKind::Input && m.signal(sid).width == 1 {
                    let low = cand.ends_with("_n") || cand.ends_with("resetn");
                    let e = m.sig_expr(sid);
                    Some(if low { m.logic_not(e) } else { e })
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| m.c_bool(false));
    let not_rst = m.logic_not(rst_act);

    // Legal retention protocol: save and restore are not asserted simultaneously.
    let both = m.logic_and(save_act, rest_act);
    let not_both = m.logic_not(both);
    let one = m.c_bool(true);
    m.properties.push(Property {
        name: "$upf_ret_mutex".into(),
        kind: PropertyKind::Assume,
        cond: not_both,
        en: one,
        source: None,
    });

    let sig_e = m.sig_expr(target_sid);
    let w = m.width(sig_e);

    // Shadow retention register captures `target` when `save_act && !rst_act`.
    let shadow_sid = m.add_signal("$upf_ret_shadow", w, false, SignalKind::Reg);
    let shadow_e = m.sig_expr(shadow_sid);
    let do_save = m.logic_and(save_act, not_rst);
    let next_shadow = m.mux(do_save, sig_e, shadow_e);
    m.ffs.push(Ff {
        signal: shadow_sid,
        next: next_shadow,
        init: None,
        clock: None,
        async_reset: None,
    });

    // Valid flag: set on `do_save`, cleared on `rst_act`.
    let valid_sid = m.add_signal("$upf_ret_valid", 1, false, SignalKind::Reg);
    m.signal_mut(valid_sid).init = Some(ConstBits::zero(1));
    let valid_e = m.sig_expr(valid_sid);
    let zero_b = m.c_bool(false);
    let next_v1 = m.logic_or(valid_e, do_save);
    let next_valid = m.mux(rst_act, zero_b, next_v1);
    m.ffs.push(Ff {
        signal: valid_sid,
        next: next_valid,
        init: Some(ConstBits::zero(1)),
        clock: None,
        async_reset: None,
    });

    // Trigger check 1 cycle after `rest_act && valid_e && !rst_act`: assert `target == prev_shadow`!
    let r1 = m.logic_and(rest_act, valid_e);
    let do_rest = m.logic_and(r1, not_rst);
    let prev_rest = m.add_signal("$upf_prev_rest", 1, false, SignalKind::Reg);
    m.signal_mut(prev_rest).init = Some(ConstBits::zero(1));
    m.ffs.push(Ff {
        signal: prev_rest,
        next: do_rest,
        init: Some(ConstBits::zero(1)),
        clock: None,
        async_reset: None,
    });
    let prev_shadow = m.add_signal("$upf_prev_shadow", w, false, SignalKind::Reg);
    m.ffs.push(Ff {
        signal: prev_shadow,
        next: shadow_e,
        init: None,
        clock: None,
        async_reset: None,
    });

    let pr_e = m.sig_expr(prev_rest);
    let ps_e = m.sig_expr(prev_shadow);
    let en = m.logic_and(pr_e, not_rst);
    let cond = m.eq(sig_e, ps_e, false);

    m.properties.push(Property {
        name: format!("upf_retention_{}", base.signal(target_sid).name),
        kind: PropertyKind::Assert,
        cond,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_power_prop(&m, cfg)
}

fn verify_clamp(
    base: &Module,
    iso_sid: SignalId,
    sense: IsoSense,
    target_sid: SignalId,
    clamp: ClampValue,
    cfg: &PowerCheckConfig,
) -> Result<PowerStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let iso_raw = m.sig_expr(iso_sid);
    let iso_b = m.to_bool(iso_raw);
    let iso_active = match sense {
        IsoSense::High => iso_b,
        IsoSense::Low => m.logic_not(iso_b),
    };

    let sig_e = m.sig_expr(target_sid);
    let w = m.width(sig_e);

    let (cond, en) = match clamp {
        ClampValue::Zero => {
            let zero = m.c_bits(ConstBits::zero(w));
            (m.eq(sig_e, zero, false), iso_active)
        }
        ClampValue::One => {
            let ones = m.c_bits(ConstBits::new(w, "1".repeat(w as usize)));
            (m.eq(sig_e, ones, false), iso_active)
        }
        ClampValue::Latch => {
            // Track `$past(sig)` and `$not_init` (`0` at cycle 0, `1` for cycles >= 1).
            let prev_sid = m.add_signal("$upf_prev_sig", w, false, SignalKind::Reg);
            m.ffs.push(Ff {
                signal: prev_sid,
                next: sig_e,
                init: None,
                clock: None,
                async_reset: None,
            });
            let not_init = m.add_signal("$upf_not_init", 1, false, SignalKind::Reg);
            m.signal_mut(not_init).init = Some(ConstBits::zero(1));
            let one_b = m.c_bool(true);
            m.ffs.push(Ff {
                signal: not_init,
                next: one_b,
                init: Some(ConstBits::zero(1)),
                clock: None,
                async_reset: None,
            });
            let prev_e = m.sig_expr(prev_sid);
            let ni_e = m.sig_expr(not_init);
            let en = m.logic_and(iso_active, ni_e);
            (m.eq(sig_e, prev_e, false), en)
        }
    };

    m.properties.push(Property {
        name: format!("upf_clamp_{}", base.signal(target_sid).name),
        kind: PropertyKind::Assert,
        cond,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_power_prop(&m, cfg)
}

fn verify_pd_off_isolated(
    base: &Module,
    off_sid: SignalId,
    iso_sid: SignalId,
    sense: IsoSense,
    cfg: &PowerCheckConfig,
) -> Result<PowerStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let off_raw = m.sig_expr(off_sid);
    let off_b = m.to_bool(off_raw);
    let iso_raw = m.sig_expr(iso_sid);
    let iso_b = m.to_bool(iso_raw);
    let iso_active = match sense {
        IsoSense::High => iso_b,
        IsoSense::Low => m.logic_not(iso_b),
    };

    m.properties.push(Property {
        name: format!("upf_pd_off_{}", base.signal(off_sid).name),
        kind: PropertyKind::Assert,
        cond: iso_active,
        en: off_b,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_power_prop(&m, cfg)
}

fn prove_power_prop(m: &Module, cfg: &PowerCheckConfig) -> Result<PowerStatus> {
    match run_kinduction(
        m,
        &ProveConfig {
            max_k: cfg.depth.min(8),
            solver: cfg.solver.clone(),
        },
    )? {
        ProveOutcome::Proved { .. } => Ok(PowerStatus::Pass),
        ProveOutcome::Fail { step, trace, .. } => Ok(PowerStatus::Fail { step, trace }),
        ProveOutcome::AssumptionsUnsat { .. } => Ok(PowerStatus::Unknown),
        ProveOutcome::Unknown { .. } if cfg.use_pdr => match run_pdr(
            m,
            &PdrConfig {
                smt_solver: cfg.solver.clone(),
                ..Default::default()
            },
        ) {
            Ok(ProveOutcome::Proved { .. }) => Ok(PowerStatus::Pass),
            Ok(ProveOutcome::Fail { step, trace, .. }) => Ok(PowerStatus::Fail { step, trace }),
            _ => Ok(PowerStatus::Unknown),
        },
        ProveOutcome::Unknown { .. } => Ok(PowerStatus::Unknown),
    }
}

fn signal_feeds_outside(m: &Module, sid: SignalId, domain_set: &HashSet<SignalId>) -> bool {
    for ca in &m.comb {
        if !domain_set.contains(&ca.signal) && expr_uses_signal(m, ca.value, sid) {
            return true;
        }
    }
    for ff in &m.ffs {
        if !domain_set.contains(&ff.signal) && expr_uses_signal(m, ff.next, sid) {
            return true;
        }
    }
    false
}

fn expr_uses_signal(m: &Module, root: ExprId, target: SignalId) -> bool {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(eid) = stack.pop() {
        if !visited.insert(eid) {
            continue;
        }
        match &m.expr(eid).kind {
            ExprKind::Signal(s) if *s == target => return true,
            ExprKind::Signal(_) | ExprKind::Const(_) => {}
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
    false
}

fn join_tcl_continuations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for raw in text.lines() {
        let trimmed_end = raw.trim_end();
        if let Some(prefix) = trimmed_end.strip_suffix('\\') {
            cur.push_str(prefix);
            cur.push(' ');
        } else {
            cur.push_str(trimmed_end);
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn strip_tcl_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        ""
    } else {
        line
    }
}

fn tokenize_tcl(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                let mut depth = 1usize;
                let mut braced = String::new();
                for c in chars.by_ref() {
                    if c == '{' {
                        depth += 1;
                        braced.push(c);
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        braced.push(c);
                    } else {
                        braced.push(c);
                    }
                }
                tokens.push(braced);
            }
            '"' => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                let mut quoted = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    quoted.push(c);
                }
                tokens.push(quoted);
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn split_list_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}
