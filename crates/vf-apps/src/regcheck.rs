use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::{ConstBits, ExprId, ExprKind, Ff, Module, Property, PropertyKind, SignalId, SignalKind};
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPolicy {
    Rw,
    Ro,
    W1c,
    W1s,
    Wo,
}

impl AccessPolicy {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "RW" | "R/W" => Ok(Self::Rw),
            "RO" | "R" => Ok(Self::Ro),
            "W1C" => Ok(Self::W1c),
            "W1S" => Ok(Self::W1s),
            "WO" | "W" => Ok(Self::Wo),
            other => bail!("unsupported register access policy '{other}' (expected RW, RO, W1C, W1S, WO)"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rw => "RW",
            Self::Ro => "RO",
            Self::W1c => "W1C",
            Self::W1s => "W1S",
            Self::Wo => "WO",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegSpec {
    pub name: String,
    pub addr: u64,
    pub width: u32,
    pub reset: u64,
    pub access: AccessPolicy,
    /// Implemented writable/active bitmask (bits outside mask are reserved).
    pub mask: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RegBusConfig {
    pub rst: Option<String>,
    pub rst_active_low: Option<bool>,
    pub wr_en: Option<String>,
    pub rd_en: Option<String>,
    pub addr: Option<String>,
    pub wdata: Option<String>,
    pub rdata: Option<String>,
    pub depth: usize,
    pub solver: String,
    pub use_pdr: bool,
}

#[derive(Debug, Clone)]
pub enum RegCheckStatus {
    Pass,
    Fail { step: usize, trace: Vec<TraceFrame> },
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RegCheckFinding {
    pub register: String,
    pub check: String,
    pub status: RegCheckStatus,
}

/// Parse CSV register map:
/// `name,addr,width,reset,access[,mask]`
pub fn parse_regmap_csv(path: &Path) -> Result<Vec<RegSpec>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read register map {}", path.display()))?;
    parse_regmap_str(&text)
}

pub fn parse_regmap_str(text: &str) -> Result<Vec<RegSpec>> {
    let mut specs = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(|c| c.trim()).collect();
        if cols.len() < 5 {
            bail!(
                "register CSV line {}: expected `name,addr,width,reset,access[,mask]`",
                idx + 1
            );
        }
        if specs.is_empty() && cols[0].eq_ignore_ascii_case("name") {
            continue;
        }
        let name = cols[0].to_string();
        let addr = parse_num(cols[1])
            .with_context(|| format!("line {}: invalid addr '{}'", idx + 1, cols[1]))?;
        let width: u32 = cols[2]
            .parse()
            .with_context(|| format!("line {}: invalid width '{}'", idx + 1, cols[2]))?;
        let reset = parse_num(cols[3])
            .with_context(|| format!("line {}: invalid reset '{}'", idx + 1, cols[3]))?;
        let access = AccessPolicy::parse(cols[4])?;
        let full_mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let mask = cols
            .get(5)
            .filter(|s| !s.is_empty())
            .map(|s| parse_num(s))
            .transpose()?
            .unwrap_or(full_mask)
            & full_mask;
        specs.push(RegSpec {
            name,
            addr,
            width,
            reset,
            access,
            mask,
        });
    }
    Ok(specs)
}

fn parse_num(s: &str) -> Result<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|e| anyhow!("{e}"))
    } else if let Some(bin) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        u64::from_str_radix(bin, 2).map_err(|e| anyhow!("{e}"))
    } else {
        s.parse::<u64>().map_err(|e| anyhow!("{e}"))
    }
}

struct ResolvedBus {
    rst: SignalId,
    rst_active_low: bool,
    wr_en: SignalId,
    rd_en: Option<SignalId>,
    addr: SignalId,
    wdata: SignalId,
    rdata: SignalId,
}

fn resolve_bus(m: &Module, cfg: &RegBusConfig) -> Result<ResolvedBus> {
    let find_any = |explicit: &Option<String>, defaults: &[&str], role: &str| -> Result<SignalId> {
        if let Some(name) = explicit {
            return m
                .find_signal(name)
                .ok_or_else(|| anyhow!("regcheck: {role} signal '{name}' not found"));
        }
        for d in defaults {
            if let Some(id) = m.find_signal(d) {
                return Ok(id);
            }
        }
        bail!("regcheck: could not auto-detect {role} signal (tried {defaults:?}); pass --{role}")
    };

    let rst = find_any(&cfg.rst, &["rst", "rst_n", "reset", "reset_n", "presetn"], "rst")?;
    let rst_name = m.signal(rst).name.to_ascii_lowercase();
    let rst_active_low = cfg
        .rst_active_low
        .unwrap_or_else(|| rst_name.ends_with("_n") || rst_name.ends_with("resetn") || rst_name == "rstn");
    let wr_en = find_any(&cfg.wr_en, &["wr_en", "we", "write_en", "pwrite"], "wr-en")?;
    let rd_en = if let Some(name) = &cfg.rd_en {
        Some(
            m.find_signal(name)
                .ok_or_else(|| anyhow!("regcheck: rd_en signal '{name}' not found"))?,
        )
    } else {
        ["rd_en", "re", "read_en"]
            .iter()
            .find_map(|d| m.find_signal(d))
    };
    let addr = find_any(&cfg.addr, &["addr", "paddr", "reg_addr"], "addr")?;
    let wdata = find_any(&cfg.wdata, &["wdata", "pwdata", "din"], "wdata")?;
    let rdata = find_any(&cfg.rdata, &["rdata", "prdata", "dout"], "rdata")?;

    Ok(ResolvedBus {
        rst,
        rst_active_low,
        wr_en,
        rd_en,
        addr,
        wdata,
        rdata,
    })
}

/// Questa Register Check equivalent: synthesizes and proves formal CSR verification
/// properties (reset values, RW/RO/W1C/W1S/WO access policies, reserved bit masking,
/// and cross-register non-aliasing) purely from the CSR bus interface + CSV register map.
pub fn run_regcheck(
    base: &Module,
    specs: &[RegSpec],
    cfg: &RegBusConfig,
) -> Result<Vec<RegCheckFinding>> {
    let bus = resolve_bus(base, cfg)?;
    let depth = if cfg.depth == 0 { 10 } else { cfg.depth };
    let solver = if cfg.solver.is_empty() {
        "z3"
    } else {
        &cfg.solver
    };

    let mut findings = Vec::new();
    for spec in specs {
        // 1. Reset value check (for readable registers).
        if spec.access != AccessPolicy::Wo {
            let status = check_reset_value(base, &bus, spec, depth, solver, cfg.use_pdr)?;
            findings.push(RegCheckFinding {
                register: spec.name.clone(),
                check: "reset".into(),
                status,
            });
        }

        // 2. Access policy + reserved bit mask check.
        let status = check_access_policy(base, &bus, spec, depth, solver, cfg.use_pdr)?;
        findings.push(RegCheckFinding {
            register: spec.name.clone(),
            check: format!("access_{}", spec.access.as_str()),
            status,
        });

        // 3. Non-aliasing check (writing to any other address does not corrupt this register).
        if spec.access != AccessPolicy::Wo && specs.len() > 1 {
            let status = check_no_aliasing(base, &bus, spec, depth, solver, cfg.use_pdr)?;
            findings.push(RegCheckFinding {
                register: spec.name.clone(),
                check: "no_alias".into(),
                status,
            });
        }
    }

    Ok(findings)
}

fn rst_active_expr(m: &mut Module, bus: &ResolvedBus) -> ExprId {
    let r = m.sig_expr(bus.rst);
    let rb = m.to_bool(r);
    if bus.rst_active_low {
        m.logic_not(rb)
    } else {
        rb
    }
}

fn rd_active_expr(m: &mut Module, bus: &ResolvedBus) -> ExprId {
    let wr = m.sig_expr(bus.wr_en);
    let not_wr = m.logic_not(wr);
    if let Some(rd) = bus.rd_en {
        let r = m.sig_expr(rd);
        m.logic_and(r, not_wr)
    } else {
        not_wr
    }
}

fn add_delay_reg(m: &mut Module, name: &str, expr: ExprId, init_zero: bool) -> ExprId {
    let w = m.width(expr);
    let sid = m.add_signal(name, w, false, SignalKind::Reg);
    let init = if init_zero {
        Some(ConstBits::zero(w))
    } else {
        None
    };
    m.signal_mut(sid).init = init.clone();
    m.ffs.push(Ff {
        signal: sid,
        next: expr,
        init,
        clock: None,
        async_reset: None,
    });
    m.sig_expr(sid)
}

fn check_reset_value(
    base: &Module,
    bus: &ResolvedBus,
    spec: &RegSpec,
    depth: usize,
    solver: &str,
    use_pdr: bool,
) -> Result<RegCheckStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let rst = rst_active_expr(&mut m, bus);
    let prev_rst = add_delay_reg(&mut m, "$reg_prev_rst", rst, true);
    let not_rst = m.logic_not(rst);
    let rd = rd_active_expr(&mut m, bus);

    let addr_e = m.sig_expr(bus.addr);
    let addr_w = m.width(addr_e);
    let target_addr = m.c_u64(addr_w, spec.addr);
    let addr_hit = m.eq(addr_e, target_addr, false);

    let en1 = m.logic_and(prev_rst, not_rst);
    let en2 = m.logic_and(en1, rd);
    let en = m.logic_and(en2, addr_hit);

    let rdata = m.sig_expr(bus.rdata);
    let rdata_resized = m.resize(rdata, spec.width, false);
    let expected = m.c_u64(spec.width, spec.reset);
    let cond = m.eq(rdata_resized, expected, false);

    m.properties.push(Property {
        name: format!("reg_reset_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_reg_prop(&m, depth, solver, use_pdr)
}

fn check_access_policy(
    base: &Module,
    bus: &ResolvedBus,
    spec: &RegSpec,
    depth: usize,
    solver: &str,
    use_pdr: bool,
) -> Result<RegCheckStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let rst = rst_active_expr(&mut m, bus);
    let not_rst = m.logic_not(rst);
    let wr = m.sig_expr(bus.wr_en);
    let wr_b = m.to_bool(wr);
    let rd = rd_active_expr(&mut m, bus);

    let addr_e = m.sig_expr(bus.addr);
    let addr_w = m.width(addr_e);
    let target_addr = m.c_u64(addr_w, spec.addr);
    let addr_hit = m.eq(addr_e, target_addr, false);

    let rdata = m.sig_expr(bus.rdata);
    let rdata_w = m.resize(rdata, spec.width, false);
    let wdata = m.sig_expr(bus.wdata);
    let wdata_w = m.resize(wdata, spec.width, false);

    if spec.access == AccessPolicy::Wo {
        // Write-Only: reading `R.addr` outside reset must always return `spec.reset` (typically 0).
        let en1 = m.logic_and(not_rst, rd);
        let en = m.logic_and(en1, addr_hit);
        let expected = m.c_u64(spec.width, spec.reset);
        let cond = m.eq(rdata_w, expected, false);
        m.properties.push(Property {
            name: format!("reg_wo_{}", spec.name),
            kind: PropertyKind::Assert,
            cond,
            en,
            source: None,
        });
        vf_ir::passes::coi_prune(&mut m);
        return prove_reg_prop(&m, depth, solver, use_pdr);
    }

    // Track: at cycle t-1 we performed a write to `R.addr` (while `!rst`), with pre-write `rdata_w` and `wdata_w`.
    let wr_hit_1 = m.logic_and(not_rst, wr_b);
    let wr_hit = m.logic_and(wr_hit_1, addr_hit);

    let prev_wr_hit = add_delay_reg(&mut m, "$reg_prev_wr_hit", wr_hit, true);
    let prev_wdata = add_delay_reg(&mut m, "$reg_prev_wdata", wdata_w, false);
    let prev_rdata = add_delay_reg(&mut m, "$reg_prev_rdata", rdata_w, false);

    // Require that reset has happened at least once before verifying reserved bits / post-reset state.
    let had_rst = m.add_signal("$reg_had_rst", 1, false, SignalKind::Reg);
    m.signal_mut(had_rst).init = Some(ConstBits::zero(1));
    let had_rst_e = m.sig_expr(had_rst);
    let next_had_rst = m.logic_or(had_rst_e, rst);
    m.ffs.push(Ff {
        signal: had_rst,
        next: next_had_rst,
        init: Some(ConstBits::zero(1)),
        clock: None,
        async_reset: None,
    });

    let en1 = m.logic_and(prev_wr_hit, not_rst);
    let en2 = m.logic_and(en1, rd);
    let en3 = m.logic_and(en2, addr_hit);
    let en = m.logic_and(en3, had_rst_e);

    let mask_c = m.c_u64(spec.width, spec.mask);
    let res_val_c = m.c_u64(spec.width, spec.reset & !spec.mask);

    let expected = match spec.access {
        AccessPolicy::Rw => {
            let masked_w = m.intern(spec.width, ExprKind::And(prev_wdata, mask_c));
            m.intern(spec.width, ExprKind::Or(masked_w, res_val_c))
        }
        AccessPolicy::Ro => prev_rdata,
        AccessPolicy::W1c => {
            let inv_w = m.not(prev_wdata);
            let cleared = m.intern(spec.width, ExprKind::And(prev_rdata, inv_w));
            let masked = m.intern(spec.width, ExprKind::And(cleared, mask_c));
            m.intern(spec.width, ExprKind::Or(masked, res_val_c))
        }
        AccessPolicy::W1s => {
            let set_bits = m.intern(spec.width, ExprKind::Or(prev_rdata, prev_wdata));
            let masked = m.intern(spec.width, ExprKind::And(set_bits, mask_c));
            m.intern(spec.width, ExprKind::Or(masked, res_val_c))
        }
        AccessPolicy::Wo => unreachable!(),
    };

    let cond = m.eq(rdata_w, expected, false);
    m.properties.push(Property {
        name: format!("reg_access_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_reg_prop(&m, depth, solver, use_pdr)
}

fn check_no_aliasing(
    base: &Module,
    bus: &ResolvedBus,
    spec: &RegSpec,
    depth: usize,
    solver: &str,
    use_pdr: bool,
) -> Result<RegCheckStatus> {
    let mut m = base.clone();
    m.properties.retain(|p| p.kind == PropertyKind::Assume);

    let rst = rst_active_expr(&mut m, bus);
    let not_rst = m.logic_not(rst);
    let wr = m.sig_expr(bus.wr_en);
    let wr_b = m.to_bool(wr);
    let rd = rd_active_expr(&mut m, bus);

    let addr_e = m.sig_expr(bus.addr);
    let addr_w = m.width(addr_e);
    let target_addr = m.c_u64(addr_w, spec.addr);
    let addr_hit = m.eq(addr_e, target_addr, false);
    let addr_other = m.logic_not(addr_hit);

    let rdata = m.sig_expr(bus.rdata);
    let rdata_w = m.resize(rdata, spec.width, false);

    // Cycle t-2: read `R.addr` (while `!rst`) -> snapshot `snap_val`.
    let rd_hit_1 = m.logic_and(not_rst, rd);
    let rd_hit = m.logic_and(rd_hit_1, addr_hit);
    let armed1 = add_delay_reg(&mut m, "$alias_armed1", rd_hit, true);
    let snap1 = add_delay_reg(&mut m, "$alias_snap1", rdata_w, false);

    // Cycle t-1: write to OTHER address (`addr != R.addr`) while `!rst`.
    let other_wr_1 = m.logic_and(not_rst, wr_b);
    let other_wr_2 = m.logic_and(other_wr_1, addr_other);
    let step2 = m.logic_and(armed1, other_wr_2);
    let armed2 = add_delay_reg(&mut m, "$alias_armed2", step2, true);
    let snap2 = add_delay_reg(&mut m, "$alias_snap2", snap1, false);

    // Cycle t: read `R.addr` again while `!rst` -> must still equal `snap2`!
    let en = m.logic_and(armed2, rd_hit);
    let cond = m.eq(rdata_w, snap2, false);

    m.properties.push(Property {
        name: format!("reg_no_alias_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en,
        source: None,
    });
    vf_ir::passes::coi_prune(&mut m);
    prove_reg_prop(&m, depth, solver, use_pdr)
}

fn prove_reg_prop(
    m: &Module,
    depth: usize,
    solver: &str,
    use_pdr: bool,
) -> Result<RegCheckStatus> {
    match run_kinduction(
        m,
        &ProveConfig {
            max_k: depth.min(8),
            solver: solver.to_string(),
        },
    )? {
        ProveOutcome::Proved { .. } => Ok(RegCheckStatus::Pass),
        ProveOutcome::Fail { step, trace, .. } => Ok(RegCheckStatus::Fail { step, trace }),
        ProveOutcome::AssumptionsUnsat { .. } => Ok(RegCheckStatus::Unknown),
        ProveOutcome::Unknown { .. } if use_pdr => match run_pdr(
            m,
            &PdrConfig {
                smt_solver: solver.to_string(),
                ..Default::default()
            },
        ) {
            Ok(ProveOutcome::Proved { .. }) => Ok(RegCheckStatus::Pass),
            Ok(ProveOutcome::Fail { step, trace, .. }) => Ok(RegCheckStatus::Fail { step, trace }),
            _ => Ok(RegCheckStatus::Unknown),
        },
        ProveOutcome::Unknown { .. } => Ok(RegCheckStatus::Unknown),
    }
}
