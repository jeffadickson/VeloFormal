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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegProtocol {
    #[default]
    Raw,
    Apb,
}

impl RegProtocol {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "raw" | "simple" | "" => Ok(Self::Raw),
            "apb" | "apb3" | "apb4" => Ok(Self::Apb),
            other => bail!("unsupported regcheck bus protocol '{other}' (expected raw or apb)"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegBusConfig {
    pub protocol: RegProtocol,
    /// Read data latency in clock cycles (`0` = combinational read mux, `1` = registered read output).
    pub read_latency: usize,
    pub rst: Option<String>,
    pub rst_active_low: Option<bool>,
    pub wr_en: Option<String>,
    pub rd_en: Option<String>,
    pub addr: Option<String>,
    pub wdata: Option<String>,
    pub rdata: Option<String>,
    pub psel: Option<String>,
    pub penable: Option<String>,
    pub pready: Option<String>,
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

/// Auto-detect and parse a register specification from `.csv`, SystemRDL 2.0 (`.rdl`),
/// or IEEE 1685 IP-XACT (`.xml`).
pub fn parse_regmap_auto(path: &Path) -> Result<Vec<RegSpec>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read register map {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rdl" => parse_systemrdl_str(&text),
        "xml" => parse_ipxact_str(&text),
        _ => parse_regmap_str(&text),
    }
}

/// Parse CSV register map:
/// `name,addr,width,reset,access[,mask]`
pub fn parse_regmap_csv(path: &Path) -> Result<Vec<RegSpec>> {
    parse_regmap_auto(path)
}

/// Parse IEEE 1685 IP-XACT XML (`<ipxact:register>` or `<spirit:register>` or `<register>`).
pub fn parse_ipxact_str(text: &str) -> Result<Vec<RegSpec>> {
    let mut specs = Vec::new();
    let mut rest = text;
    while let Some((reg_body, after)) = extract_xml_tag(rest, "register") {
        rest = after;
        let name = extract_xml_text(reg_body, "name")
            .ok_or_else(|| anyhow!("IP-XACT <register> missing <name>"))?;
        let addr_s = extract_xml_text(reg_body, "addressOffset")
            .ok_or_else(|| anyhow!("IP-XACT register '{name}' missing <addressOffset>"))?;
        let addr = parse_sv_or_c_num(&addr_s)?;
        let width: u32 = extract_xml_text(reg_body, "size")
            .map(|s| parse_sv_or_c_num(&s))
            .transpose()?
            .unwrap_or(32) as u32;
        let reg_reset = extract_xml_text(reg_body, "resetValue")
            .or_else(|| extract_xml_text(reg_body, "value"))
            .map(|s| parse_sv_or_c_num(&s))
            .transpose()?
            .unwrap_or(0);
        let reg_access_str = extract_xml_text(reg_body, "access").unwrap_or_default();

        // Parse <field> children if present to build exact bitmask, reset, and access policy.
        let mut mask: u64 = 0;
        let mut field_reset: u64 = 0;
        let mut saw_field = false;
        let mut access = map_ipxact_access(&reg_access_str, "");
        let mut f_rest = reg_body;
        while let Some((f_body, f_after)) = extract_xml_tag(f_rest, "field") {
            f_rest = f_after;
            saw_field = true;
            let bit_off = extract_xml_text(f_body, "bitOffset")
                .map(|s| parse_sv_or_c_num(&s))
                .transpose()?
                .unwrap_or(0) as u32;
            let bit_w = extract_xml_text(f_body, "bitWidth")
                .map(|s| parse_sv_or_c_num(&s))
                .transpose()?
                .unwrap_or(1) as u32;
            let f_mask = if bit_w >= 64 {
                u64::MAX
            } else {
                ((1u64 << bit_w) - 1) << bit_off
            };
            mask |= f_mask;
            if let Some(rv) = extract_xml_text(f_body, "resetValue")
                .or_else(|| extract_xml_text(f_body, "value"))
            {
                let v = parse_sv_or_c_num(&rv)?;
                field_reset |= (v << bit_off) & f_mask;
            }
            let fa = extract_xml_text(f_body, "access").unwrap_or_else(|| reg_access_str.clone());
            let mwv = extract_xml_text(f_body, "modifiedWriteValue").unwrap_or_default();
            access = map_ipxact_access(&fa, &mwv);
        }

        let full_mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        if !saw_field {
            mask = full_mask;
        }
        let reset = if reg_reset != 0 { reg_reset } else { field_reset };
        specs.push(RegSpec {
            name,
            addr,
            width,
            reset: reset & full_mask,
            access,
            mask: mask & full_mask,
        });
    }
    if specs.is_empty() {
        bail!("IP-XACT XML contained no <register> definitions");
    }
    Ok(specs)
}

fn map_ipxact_access(access: &str, mod_write: &str) -> AccessPolicy {
    match mod_write.trim().to_ascii_lowercase().as_str() {
        "onetoclear" | "w1c" => return AccessPolicy::W1c,
        "onetoset" | "w1s" => return AccessPolicy::W1s,
        _ => {}
    }
    match access.trim().to_ascii_lowercase().as_str() {
        "read-only" | "ro" => AccessPolicy::Ro,
        "write-only" | "wo" => AccessPolicy::Wo,
        "read-write" | "rw" | "" => AccessPolicy::Rw,
        _ => AccessPolicy::Rw,
    }
}

fn extract_xml_tag<'a>(src: &'a str, local_name: &str) -> Option<(&'a str, &'a str)> {
    // Match `<local_name ...>` or `<prefix:local_name ...>` and find matching `</...local_name>`.
    let mut search_from = 0;
    while let Some(rel) = src[search_from..].find('<') {
        let start = search_from + rel;
        let after_lt = &src[start + 1..];
        if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
            search_from = start + 1;
            continue;
        }
        let tag_end = after_lt.find('>')?;
        let tag_hdr = &after_lt[..tag_end];
        let tag_name = tag_hdr.split_whitespace().next().unwrap_or("");
        let stripped = tag_name.rsplit(':').next().unwrap_or(tag_name);
        if stripped == local_name {
            let body_start = start + 1 + tag_end + 1;
            let close_pat1 = format!("</{tag_name}>");
            let close_pat2 = format!("</{local_name}>");
            if let Some(rel_end) = src[body_start..].find(&close_pat1) {
                let body_end = body_start + rel_end;
                return Some((&src[body_start..body_end], &src[body_end + close_pat1.len()..]));
            }
            if let Some(rel_end) = src[body_start..].find(&close_pat2) {
                let body_end = body_start + rel_end;
                return Some((&src[body_start..body_end], &src[body_end + close_pat2.len()..]));
            }
            return None;
        }
        search_from = start + 1;
    }
    None
}

fn extract_xml_text(src: &str, local_name: &str) -> Option<String> {
    let (body, _) = extract_xml_tag(src, local_name)?;
    Some(body.trim().to_string())
}

/// Parse SystemRDL 2.0 (`.rdl`) `reg { ... } NAME @ 0xADDR;` blocks.
pub fn parse_systemrdl_str(text: &str) -> Result<Vec<RegSpec>> {
    let cleaned = strip_c_comments(text);
    let mut specs = Vec::new();
    let mut i = 0usize;
    let bytes = cleaned.as_bytes();

    while i < bytes.len() {
        // Look for standalone keyword `reg` followed by `{`.
        if let Some(rel) = cleaned[i..].find("reg") {
            let pos = i + rel;
            let before_ok = pos == 0
                || !(bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_');
            let after_pos = pos + 3;
            let after_ok = after_pos < bytes.len()
                && !(bytes[after_pos].is_ascii_alphanumeric() || bytes[after_pos] == b'_');
            if !before_ok || !after_ok {
                i = after_pos;
                continue;
            }
            let tail = cleaned[after_pos..].trim_start();
            // Optional named reg type before `{`: `reg my_t { ... }` or anonymous `reg { ... }`
            let Some(brace_off) = tail.find('{') else {
                i = after_pos;
                continue;
            };
            let between = tail[..brace_off].trim();
            if between.contains(';') || between.contains('=') {
                i = after_pos;
                continue;
            }
            let body_start = cleaned.len() - tail.len() + brace_off + 1;
            let Some(body_end) = find_matching_brace(&cleaned, body_start) else {
                bail!("unterminated SystemRDL `reg {{ ... }}` block");
            };
            let reg_body = &cleaned[body_start..body_end];
            let post = &cleaned[body_end + 1..];
            let Some(semi) = post.find(';') else {
                bail!("expected `;` after SystemRDL `reg {{ ... }}` declaration");
            };
            let decl = post[..semi].trim();
            // Parse `NAME @ 0xADDR`
            if let Some((name_part, addr_part)) = decl.split_once('@') {
                let name = name_part.trim().to_string();
                let addr = parse_sv_or_c_num(addr_part.trim())?;
                specs.push(parse_rdl_reg_body(&name, addr, reg_body)?);
            }
            i = body_end + 1 + semi + 1;
        } else {
            break;
        }
    }

    if specs.is_empty() {
        bail!("SystemRDL input contained no `reg {{ ... }} NAME @ ADDR;` definitions");
    }
    Ok(specs)
}

fn parse_rdl_reg_body(name: &str, addr: u64, body: &str) -> Result<RegSpec> {
    let mut width: u32 = 32;
    for stmt in body.split(';') {
        let s = stmt.trim();
        if let Some(rhs) = s.strip_prefix("regwidth") {
            if let Some(val) = rhs.trim().strip_prefix('=') {
                width = parse_sv_or_c_num(val.trim())? as u32;
            }
        }
    }

    let mut mask: u64 = 0;
    let mut reset: u64 = 0;
    let mut access = AccessPolicy::Rw;
    let mut saw_field = false;

    let mut pos = 0usize;
    while let Some(rel) = body[pos..].find("field") {
        let fpos = pos + rel;
        let after = fpos + 5;
        let tail = body[after..].trim_start();
        let Some(boff) = tail.find('{') else {
            pos = after;
            continue;
        };
        let fbody_start = body.len() - tail.len() + boff + 1;
        let Some(fbody_end) = find_matching_brace(body, fbody_start) else {
            bail!("unterminated SystemRDL `field {{ ... }}` in register '{name}'");
        };
        let fbody = &body[fbody_start..fbody_end];
        let post = &body[fbody_end + 1..];
        let Some(semi) = post.find(';') else {
            bail!("missing `;` after SystemRDL field in register '{name}'");
        };
        let fdecl = post[..semi].trim();
        saw_field = true;

        // Parse bit range `[hi:lo]` or `[bit]` and optional `= reset_val`
        let (hi, lo, inline_reset) = parse_rdl_field_decl(fdecl)?;
        let bw = hi - lo + 1;
        let fmask = if bw >= 64 {
            u64::MAX
        } else {
            ((1u64 << bw) - 1) << lo
        };
        mask |= fmask;

        let mut f_reset = inline_reset;
        let mut sw = String::new();
        let mut onwrite = String::new();
        for prop in fbody.split(';') {
            let p = prop.trim();
            if let Some((k, v)) = p.split_once('=') {
                match k.trim() {
                    "reset" => f_reset = Some(parse_sv_or_c_num(v.trim())?),
                    "sw" => sw = v.trim().to_ascii_lowercase(),
                    "onwrite" => onwrite = v.trim().to_ascii_lowercase(),
                    _ => {}
                }
            }
        }
        if let Some(rv) = f_reset {
            reset |= (rv << lo) & fmask;
        }
        access = match onwrite.as_str() {
            "woclr" => AccessPolicy::W1c,
            "woset" => AccessPolicy::W1s,
            _ => match sw.as_str() {
                "r" => AccessPolicy::Ro,
                "w" => AccessPolicy::Wo,
                _ => AccessPolicy::Rw,
            },
        };

        pos = fbody_end + 1 + semi + 1;
    }

    let full_mask = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    if !saw_field {
        mask = full_mask;
    }
    Ok(RegSpec {
        name: name.to_string(),
        addr,
        width,
        reset: reset & full_mask,
        access,
        mask: mask & full_mask,
    })
}

fn parse_rdl_field_decl(decl: &str) -> Result<(u32, u32, Option<u64>)> {
    let (lhs, inline_reset) = if let Some((l, r)) = decl.split_once('=') {
        (l.trim(), Some(parse_sv_or_c_num(r.trim())?))
    } else {
        (decl.trim(), None)
    };
    if let (Some(lb), Some(rb)) = (lhs.find('['), lhs.find(']')) {
        let range = lhs[lb + 1..rb].trim();
        if let Some((a, b)) = range.split_once(':') {
            let hi = a.trim().parse::<u32>()?;
            let lo = b.trim().parse::<u32>()?;
            return Ok((hi.max(lo), hi.min(lo), inline_reset));
        }
        let bit = range.parse::<u32>()?;
        return Ok((bit, bit, inline_reset));
    }
    Ok((0, 0, inline_reset))
}

fn find_matching_brace(s: &str, open_body_start: usize) -> Option<usize> {
    let mut depth = 1usize;
    for (off, ch) in s[open_body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_body_start + off);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_c_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn parse_sv_or_c_num(s: &str) -> Result<u64> {
    let clean = s.trim().replace('_', "");
    if let Some((_, rhs)) = clean.split_once('\'') {
        let rhs = rhs.trim();
        let (base, digits) = if let Some(d) = rhs.strip_prefix('h').or_else(|| rhs.strip_prefix('H')) {
            (16, d)
        } else if let Some(d) = rhs.strip_prefix('d').or_else(|| rhs.strip_prefix('D')) {
            (10, d)
        } else if let Some(d) = rhs.strip_prefix('b').or_else(|| rhs.strip_prefix('B')) {
            (2, d)
        } else {
            (10, rhs)
        };
        return u64::from_str_radix(digits, base).map_err(|e| anyhow!("invalid literal '{s}': {e}"));
    }
    parse_num(&clean)
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
    protocol: RegProtocol,
    read_latency: usize,
    rst: SignalId,
    rst_active_low: bool,
    wr_en: SignalId,
    rd_en: Option<SignalId>,
    psel: Option<SignalId>,
    penable: Option<SignalId>,
    pready: Option<SignalId>,
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
    let rst_active_low = cfg.rst_active_low.unwrap_or_else(|| {
        rst_name.ends_with("_n") || rst_name.ends_with("resetn") || rst_name == "rstn"
    });
    let wr_en = find_any(&cfg.wr_en, &["wr_en", "we", "write_en", "pwrite"], "wr-en")?;
    let rd_en = if let Some(name) = &cfg.rd_en {
        Some(
            m.find_signal(name)
                .ok_or_else(|| anyhow!("regcheck: rd_en signal '{name}' not found"))?,
        )
    } else if cfg.protocol == RegProtocol::Raw {
        ["rd_en", "re", "read_en"]
            .iter()
            .find_map(|d| m.find_signal(d))
    } else {
        None
    };

    let (psel, penable, pready) = if cfg.protocol == RegProtocol::Apb {
        let ps = find_any(&cfg.psel, &["psel", "pselx", "PSEL"], "psel")?;
        let pe = find_any(&cfg.penable, &["penable", "PENABLE"], "penable")?;
        let pr = if let Some(name) = &cfg.pready {
            Some(
                m.find_signal(name)
                    .ok_or_else(|| anyhow!("regcheck: pready signal '{name}' not found"))?,
            )
        } else {
            ["pready", "PREADY"].iter().find_map(|d| m.find_signal(d))
        };
        (Some(ps), Some(pe), pr)
    } else {
        (None, None, None)
    };

    let addr = find_any(&cfg.addr, &["addr", "paddr", "PADDR", "reg_addr"], "addr")?;
    let wdata = find_any(&cfg.wdata, &["wdata", "pwdata", "PWDATA", "din"], "wdata")?;
    let rdata = find_any(&cfg.rdata, &["rdata", "prdata", "PRDATA", "dout"], "rdata")?;

    Ok(ResolvedBus {
        protocol: cfg.protocol,
        read_latency: cfg.read_latency.min(1),
        rst,
        rst_active_low,
        wr_en,
        rd_en,
        psel,
        penable,
        pready,
        addr,
        wdata,
        rdata,
    })
}

/// Questa Register Check equivalent: synthesizes and proves formal CSR verification
/// properties (reset values, RW/RO/W1C/W1S/WO access policies, reserved bit masking,
/// and cross-register non-aliasing) purely from the CSR bus interface + CSV/SystemRDL/IP-XACT map.
/// Supports both Raw CSR buses and native AMBA APB3/APB4 (`--protocol apb`) + 0/1-cycle read latency.
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

fn add_protocol_assumptions(m: &mut Module, bus: &ResolvedBus) {
    if bus.protocol == RegProtocol::Apb {
        // Legal AMBA APB: `penable` may only be asserted when `psel` is high.
        if let (Some(psel), Some(penable)) = (bus.psel, bus.penable) {
            let ps = m.sig_expr(psel);
            let pe = m.sig_expr(penable);
            let ps_b = m.to_bool(ps);
            let pe_b = m.to_bool(pe);
            m.properties.push(Property {
                name: "$apb_penable_implies_psel".into(),
                kind: PropertyKind::Assume,
                cond: ps_b,
                en: pe_b,
                source: None,
            });
        }
    }
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

fn wr_active_expr(m: &mut Module, bus: &ResolvedBus) -> ExprId {
    let wr = m.sig_expr(bus.wr_en);
    let wr_b = m.to_bool(wr);
    if bus.protocol == RegProtocol::Apb {
        let ps = m.sig_expr(bus.psel.unwrap());
        let pe = m.sig_expr(bus.penable.unwrap());
        let xfer = m.logic_and(ps, pe);
        let xfer = if let Some(pr) = bus.pready {
            let pr_e = m.sig_expr(pr);
            m.logic_and(xfer, pr_e)
        } else {
            xfer
        };
        m.logic_and(xfer, wr_b)
    } else {
        wr_b
    }
}

fn rd_active_expr(m: &mut Module, bus: &ResolvedBus) -> ExprId {
    let wr = m.sig_expr(bus.wr_en);
    let not_wr = m.logic_not(wr);
    if bus.protocol == RegProtocol::Apb {
        let ps = m.sig_expr(bus.psel.unwrap());
        let pe = m.sig_expr(bus.penable.unwrap());
        let xfer = m.logic_and(ps, pe);
        let xfer = if let Some(pr) = bus.pready {
            let pr_e = m.sig_expr(pr);
            m.logic_and(xfer, pr_e)
        } else {
            xfer
        };
        m.logic_and(xfer, not_wr)
    } else if let Some(rd) = bus.rd_en {
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

/// Apply optional 1-cycle read latency alignment to `(en, expected)` before comparing against `rdata_w`.
fn align_read_check(
    m: &mut Module,
    bus: &ResolvedBus,
    not_rst: ExprId,
    en: ExprId,
    expected: ExprId,
    tag: &str,
) -> (ExprId, ExprId) {
    if bus.read_latency == 0 {
        (en, expected)
    } else {
        let en_d = add_delay_reg(m, &format!("$reg_rden_d_{tag}"), en, true);
        let exp_d = add_delay_reg(m, &format!("$reg_exp_d_{tag}"), expected, false);
        let en_final = m.logic_and(en_d, not_rst);
        (en_final, exp_d)
    }
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
    add_protocol_assumptions(&mut m, bus);

    let rst = rst_active_expr(&mut m, bus);
    let not_rst = m.logic_not(rst);
    let wr = wr_active_expr(&mut m, bus);
    let not_wr = m.logic_not(wr);
    let rd = rd_active_expr(&mut m, bus);

    let prev_rst = add_delay_reg(&mut m, "$reg_prev_rst", rst, true);
    // Allow 1 non-write Setup cycle after reset in APB mode.
    let rst_ready = if bus.protocol == RegProtocol::Apb {
        let quiet_after_rst = m.logic_and(prev_rst, not_rst);
        let quiet_after_rst = m.logic_and(quiet_after_rst, not_wr);
        let prev2_rst = add_delay_reg(&mut m, "$reg_prev2_rst", quiet_after_rst, true);
        m.logic_or(prev_rst, prev2_rst)
    } else {
        prev_rst
    };

    let addr_e = m.sig_expr(bus.addr);
    let addr_w = m.width(addr_e);
    let target_addr = m.c_u64(addr_w, spec.addr);
    let addr_hit = m.eq(addr_e, target_addr, false);

    let en1 = m.logic_and(rst_ready, not_rst);
    let en2 = m.logic_and(en1, rd);
    let en = m.logic_and(en2, addr_hit);

    let rdata = m.sig_expr(bus.rdata);
    let rdata_resized = m.resize(rdata, spec.width, false);
    let expected = m.c_u64(spec.width, spec.reset);
    let (en_aligned, exp_aligned) = align_read_check(&mut m, bus, not_rst, en, expected, "rst");
    let cond = m.eq(rdata_resized, exp_aligned, false);

    m.properties.push(Property {
        name: format!("reg_reset_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en: en_aligned,
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
    add_protocol_assumptions(&mut m, bus);

    let rst = rst_active_expr(&mut m, bus);
    let not_rst = m.logic_not(rst);
    let wr_b = wr_active_expr(&mut m, bus);
    let not_wr = m.logic_not(wr_b);
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
        let en1 = m.logic_and(not_rst, rd);
        let en = m.logic_and(en1, addr_hit);
        let expected = m.c_u64(spec.width, spec.reset);
        let (en_aligned, exp_aligned) = align_read_check(&mut m, bus, not_rst, en, expected, "wo");
        let cond = m.eq(rdata_w, exp_aligned, false);
        m.properties.push(Property {
            name: format!("reg_wo_{}", spec.name),
            kind: PropertyKind::Assert,
            cond,
            en: en_aligned,
            source: None,
        });
        vf_ir::passes::coi_prune(&mut m);
        return prove_reg_prop(&m, depth, solver, use_pdr);
    }

    // Track whether reset has happened at least once before verifying reserved bits.
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

    let mask_c = m.c_u64(spec.width, spec.mask);
    let res_val_c = m.c_u64(spec.width, spec.reset & !spec.mask);

    let wr_hit_1 = m.logic_and(not_rst, wr_b);
    let wr_hit = m.logic_and(wr_hit_1, addr_hit);
    let rd_hit_1 = m.logic_and(not_rst, rd);
    let rd_hit = m.logic_and(rd_hit_1, addr_hit);

    // For RW, pre-write rdata is not needed; check Write -> [optional Setup] -> Read!
    if spec.access == AccessPolicy::Rw {
        let prev_wr_hit = add_delay_reg(&mut m, "$reg_prev_wr_hit", wr_hit, true);
        let prev_wdata = add_delay_reg(&mut m, "$reg_prev_wdata", wdata_w, false);

        let (armed, eff_wdata) = if bus.protocol == RegProtocol::Apb {
            // Allow 1 idle/Setup cycle between APB Write Access and APB Read Access.
            let q = m.logic_and(prev_wr_hit, not_rst);
            let q = m.logic_and(q, not_wr);
            let prev2_wr_hit = add_delay_reg(&mut m, "$reg_prev2_wr_hit", q, true);
            let prev2_wdata = add_delay_reg(&mut m, "$reg_prev2_wdata", prev_wdata, false);
            let armed = m.logic_or(prev_wr_hit, prev2_wr_hit);
            let wsel = m.mux(prev_wr_hit, prev_wdata, prev2_wdata);
            (armed, wsel)
        } else {
            (prev_wr_hit, prev_wdata)
        };

        let en1 = m.logic_and(armed, rd_hit);
        let en = m.logic_and(en1, had_rst_e);

        let masked_w = m.intern(spec.width, ExprKind::And(eff_wdata, mask_c));
        let expected = m.intern(spec.width, ExprKind::Or(masked_w, res_val_c));
        let (en_aligned, exp_aligned) = align_read_check(&mut m, bus, not_rst, en, expected, "rw");
        let cond = m.eq(rdata_w, exp_aligned, false);

        m.properties.push(Property {
            name: format!("reg_access_{}", spec.name),
            kind: PropertyKind::Assert,
            cond,
            en: en_aligned,
            source: None,
        });
        vf_ir::passes::coi_prune(&mut m);
        return prove_reg_prop(&m, depth, solver, use_pdr);
    }

    // For RO, W1C, W1S:
    // - On Raw with read_latency == 0: combinational read mux exposes pre-write value on the write cycle.
    // - On APB or read_latency == 1: use explicit Read -> [Setup] -> Write -> [Setup] -> Read sequence!
    let (en, prev_rdata, prev_wdata) = if bus.protocol == RegProtocol::Raw && bus.read_latency == 0
    {
        let prev_wr_hit = add_delay_reg(&mut m, "$reg_prev_wr_hit", wr_hit, true);
        let prev_wdata = add_delay_reg(&mut m, "$reg_prev_wdata", wdata_w, false);
        let prev_rdata = add_delay_reg(&mut m, "$reg_prev_rdata", rdata_w, false);
        let en1 = m.logic_and(prev_wr_hit, rd_hit);
        let en = m.logic_and(en1, had_rst_e);
        (en, prev_rdata, prev_wdata)
    } else if bus.protocol == RegProtocol::Apb {
        // APB Read Access -> 1 Setup cycle -> Write Access -> 1 Setup cycle -> Read Access
        let r1 = add_delay_reg(&mut m, "$apb_r1", rd_hit, true);
        let s1 = add_delay_reg(&mut m, "$apb_s1", rdata_w, false);
        let q1 = m.logic_and(r1, not_rst);
        let q1 = m.logic_and(q1, not_wr);
        let r2 = add_delay_reg(&mut m, "$apb_r2", q1, true);
        let s2 = add_delay_reg(&mut m, "$apb_s2", s1, false);

        let pre_ok = m.logic_or(r1, r2);
        let pre_val = m.mux(r1, s1, s2);

        let w_step = m.logic_and(pre_ok, wr_hit);
        let w1 = add_delay_reg(&mut m, "$apb_w1", w_step, true);
        let ws1 = add_delay_reg(&mut m, "$apb_ws1", pre_val, false);
        let wd1 = add_delay_reg(&mut m, "$apb_wd1", wdata_w, false);

        let q2 = m.logic_and(w1, not_rst);
        let q2 = m.logic_and(q2, not_wr);
        let w2 = add_delay_reg(&mut m, "$apb_w2", q2, true);
        let ws2 = add_delay_reg(&mut m, "$apb_ws2", ws1, false);
        let wd2 = add_delay_reg(&mut m, "$apb_wd2", wd1, false);

        let post_ok = m.logic_or(w1, w2);
        let snap_pre = m.mux(w1, ws1, ws2);
        let snap_wd = m.mux(w1, wd1, wd2);
        let en1 = m.logic_and(post_ok, rd_hit);
        let en = m.logic_and(en1, had_rst_e);
        (en, snap_pre, snap_wd)
    } else {
        // Raw with read_latency == 1:
        // t-3: rd_hit -> t-2: wr_hit & capture rdata_w (result of t-3 read!) -> t-1: rd_hit -> t: check rdata_w
        let r1 = add_delay_reg(&mut m, "$rl_r1", rd_hit, true);
        let w_step = m.logic_and(r1, wr_hit);
        let w1 = add_delay_reg(&mut m, "$rl_w1", w_step, true);
        let snap_pre = add_delay_reg(&mut m, "$rl_pre", rdata_w, false);
        let snap_wd = add_delay_reg(&mut m, "$rl_wd", wdata_w, false);
        let en1 = m.logic_and(w1, rd_hit);
        let en = m.logic_and(en1, had_rst_e);
        (en, snap_pre, snap_wd)
    };

    let expected = match spec.access {
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
        AccessPolicy::Rw | AccessPolicy::Wo => unreachable!(),
    };

    let (en_aligned, exp_aligned) = align_read_check(&mut m, bus, not_rst, en, expected, "acc");
    let cond = m.eq(rdata_w, exp_aligned, false);
    m.properties.push(Property {
        name: format!("reg_access_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en: en_aligned,
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
    add_protocol_assumptions(&mut m, bus);

    let rst = rst_active_expr(&mut m, bus);
    let not_rst = m.logic_not(rst);
    let wr_b = wr_active_expr(&mut m, bus);
    let not_wr = m.logic_not(wr_b);
    let rd = rd_active_expr(&mut m, bus);

    let addr_e = m.sig_expr(bus.addr);
    let addr_w = m.width(addr_e);
    let target_addr = m.c_u64(addr_w, spec.addr);
    let addr_hit = m.eq(addr_e, target_addr, false);
    let addr_other = m.logic_not(addr_hit);

    let rdata = m.sig_expr(bus.rdata);
    let rdata_w = m.resize(rdata, spec.width, false);

    let rd_hit_1 = m.logic_and(not_rst, rd);
    let rd_hit = m.logic_and(rd_hit_1, addr_hit);
    let other_wr_1 = m.logic_and(not_rst, wr_b);
    let other_wr = m.logic_and(other_wr_1, addr_other);

    let (en, snap_expected) = if bus.protocol == RegProtocol::Raw && bus.read_latency == 0 {
        let armed1 = add_delay_reg(&mut m, "$alias_armed1", rd_hit, true);
        let snap1 = add_delay_reg(&mut m, "$alias_snap1", rdata_w, false);
        let step2 = m.logic_and(armed1, other_wr);
        let armed2 = add_delay_reg(&mut m, "$alias_armed2", step2, true);
        let snap2 = add_delay_reg(&mut m, "$alias_snap2", snap1, false);
        let en = m.logic_and(armed2, rd_hit);
        (en, snap2)
    } else if bus.protocol == RegProtocol::Apb {
        let r1 = add_delay_reg(&mut m, "$alias_r1", rd_hit, true);
        let s1 = add_delay_reg(&mut m, "$alias_s1", rdata_w, false);
        let q1 = m.logic_and(r1, not_rst);
        let q1 = m.logic_and(q1, not_wr);
        let r2 = add_delay_reg(&mut m, "$alias_r2", q1, true);
        let s2 = add_delay_reg(&mut m, "$alias_s2", s1, false);
        let pre_ok = m.logic_or(r1, r2);
        let pre_val = m.mux(r1, s1, s2);

        let step2 = m.logic_and(pre_ok, other_wr);
        let w1 = add_delay_reg(&mut m, "$alias_w1", step2, true);
        let ws1 = add_delay_reg(&mut m, "$alias_ws1", pre_val, false);
        let q2 = m.logic_and(w1, not_rst);
        let q2 = m.logic_and(q2, not_wr);
        let w2 = add_delay_reg(&mut m, "$alias_w2", q2, true);
        let ws2 = add_delay_reg(&mut m, "$alias_ws2", ws1, false);
        let post_ok = m.logic_or(w1, w2);
        let snap = m.mux(w1, ws1, ws2);
        let en = m.logic_and(post_ok, rd_hit);
        (en, snap)
    } else {
        // Raw with read_latency == 1
        let r1 = add_delay_reg(&mut m, "$alias_rl_r1", rd_hit, true);
        let step2 = m.logic_and(r1, other_wr);
        let w1 = add_delay_reg(&mut m, "$alias_rl_w1", step2, true);
        let snap = add_delay_reg(&mut m, "$alias_rl_snap", rdata_w, false);
        let en = m.logic_and(w1, rd_hit);
        (en, snap)
    };

    let (en_aligned, exp_aligned) =
        align_read_check(&mut m, bus, not_rst, en, snap_expected, "alias");
    let cond = m.eq(rdata_w, exp_aligned, false);

    m.properties.push(Property {
        name: format!("reg_no_alias_{}", spec.name),
        kind: PropertyKind::Assert,
        cond,
        en: en_aligned,
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
