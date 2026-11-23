use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::live::{run_live, LiveConfig, LiveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::passes::{async2sync, chformal, clk2fflogic, has_multiple_clocks, ChformalAction};
use vf_ir::{Module, PropertyKind};
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::vcd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbyMode {
    Bmc,
    Prove,
    Cover,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SbyEngine {
    SmtBmc { solver: String },
    AbcPdr,
}

#[derive(Debug, Clone)]
pub enum ScriptPass {
    Chformal {
        filter: Option<PropertyKind>,
        action: ChformalAction,
    },
    Async2Sync,
    Clk2FfLogic,
}

#[derive(Debug, Clone)]
pub struct SbyTaskConfig {
    pub name: String,
    pub mode: SbyMode,
    pub depth: usize,
    pub skip: usize,
    pub append: usize,
    pub multiclock: bool,
    pub expect: HashSet<String>,
    pub engines: Vec<SbyEngine>,
    pub passes: Vec<ScriptPass>,
    pub compile: CompileOptions,
    pub inline_files: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SbyTaskResult {
    pub task: String,
    pub status: String,
    pub expected_ok: bool,
    pub detail: String,
    pub vcd_files: Vec<PathBuf>,
}

/// Parse a SymbiYosys `.sby` file (including `[tasks]`, conditional tags, `[file <name>]`
/// inline files, `[options]`, `[engines]`, `[script]`, `[files]`) into per-task configs.
pub fn parse_sby_tasks(path: &Path) -> Result<Vec<SbyTaskConfig>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read .sby file {}", path.display()))?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    // Pass 1: split into raw sections (preserving `[file <name>]` verbatim!).
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut cur_section: Option<String> = None;
    let mut cur_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[") {
            if let Some(sec) = cur_section.take() {
                sections.push((sec, std::mem::take(&mut cur_lines)));
            }
            cur_section = Some(trimmed[1..trimmed.len() - 1].trim().to_string());
        } else if cur_section.is_some() {
            cur_lines.push(raw_line.to_string());
        }
    }
    if let Some(sec) = cur_section {
        sections.push((sec, cur_lines));
    }

    // Collect tasks from `[tasks]` (if omitted, create a single default task).
    let mut tasks: Vec<(String, HashSet<String>)> = Vec::new();
    for (sec, lines) in &sections {
        if sec.eq_ignore_ascii_case("tasks") {
            for raw in lines {
                let line = strip_comment(raw);
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                let name = parts[0].to_string();
                let mut tags: HashSet<String> = parts.iter().map(|s| s.to_string()).collect();
                tags.insert(name.clone());
                tasks.push((name, tags));
            }
        }
    }
    if tasks.is_empty() {
        tasks.push(("default".to_string(), HashSet::new()));
    }

    // Build config for each task.
    let mut configs = Vec::with_capacity(tasks.len());
    for (task_name, tags) in tasks {
        let mut mode = SbyMode::Bmc;
        let mut depth = 20usize;
        let mut skip = 0usize;
        let mut append = 0usize;
        let mut multiclock = false;
        let mut expect: HashSet<String> = ["PASS".to_string()].into_iter().collect();
        let mut engines: Vec<SbyEngine> = Vec::new();
        let mut passes: Vec<ScriptPass> = Vec::new();
        let mut top: Option<String> = None;
        let mut files: Vec<PathBuf> = Vec::new();
        let mut defines: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();
        let mut inline_files: HashMap<String, String> = HashMap::new();

        for (sec, lines) in &sections {
            let sec_lower = sec.to_ascii_lowercase();
            if let Some(rest) = sec.strip_prefix("file ") {
                let fname = rest.trim().to_string();
                let mut content = lines.join("\n");
                content.push('\n');
                inline_files.insert(fname, content);
                continue;
            }
            match sec_lower.as_str() {
                "options" => {
                    for raw in lines {
                        let Some(line) = match_tag_line(strip_comment(raw), &tags) else {
                            continue;
                        };
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        match parts.as_slice() {
                            ["mode", "bmc"] => mode = SbyMode::Bmc,
                            ["mode", "prove"] => mode = SbyMode::Prove,
                            ["mode", "cover"] => mode = SbyMode::Cover,
                            ["mode", "live"] => mode = SbyMode::Live,
                            ["mode", other] => bail!("unsupported .sby mode '{other}'"),
                            ["depth", n] => depth = n.parse().context("invalid .sby depth")?,
                            ["skip", n] => skip = n.parse().context("invalid .sby skip")?,
                            ["append", n] => append = n.parse().context("invalid .sby append")?,
                            ["multiclock", "on"] => multiclock = true,
                            ["multiclock", "off"] => multiclock = false,
                            ["expect", rest @ ..] => {
                                expect.clear();
                                for tok in rest {
                                    for item in tok.split(',') {
                                        let up = item.trim().to_ascii_uppercase();
                                        if !up.is_empty() {
                                            expect.insert(up);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "engines" => {
                    for raw in lines {
                        let Some(line) = match_tag_line(strip_comment(raw), &tags) else {
                            continue;
                        };
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.is_empty() {
                            continue;
                        }
                        if parts[0] == "abc" && parts.get(1).copied() == Some("pdr") {
                            engines.push(SbyEngine::AbcPdr);
                        } else if parts[0] == "smtbmc" || parts[0] == "btor" {
                            let mut solver = "z3".to_string();
                            for &tok in &parts[1..] {
                                if tok == "z3" || tok == "bitwuzla" || tok == "cvc5" {
                                    solver = tok.to_string();
                                }
                            }
                            engines.push(SbyEngine::SmtBmc { solver });
                        }
                    }
                }
                "script" => {
                    for raw in lines {
                        let Some(line) = match_tag_line(strip_comment(raw), &tags) else {
                            continue;
                        };
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.is_empty() {
                            continue;
                        }
                        match parts[0] {
                            "prep" | "hierarchy" => {
                                let mut i = 1;
                                while i < parts.len() {
                                    if parts[i] == "-top" && i + 1 < parts.len() {
                                        top = Some(parts[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                            }
                            "read" | "read_verilog" => {
                                for &tok in &parts[1..] {
                                    if let Some(def) = tok.strip_prefix("-D") {
                                        defines.push(def.to_string());
                                    }
                                }
                            }
                            "chparam" => {
                                let mut i = 1;
                                while i < parts.len() {
                                    if parts[i] == "-set" && i + 2 < parts.len() {
                                        params.push(format!("{}={}", parts[i + 1], parts[i + 2]));
                                        i += 3;
                                    } else {
                                        i += 1;
                                    }
                                }
                            }
                            "async2sync" => passes.push(ScriptPass::Async2Sync),
                            "clk2fflogic" => passes.push(ScriptPass::Clk2FfLogic),
                            "chformal" => {
                                if let Some(pass) = parse_chformal(&parts[1..]) {
                                    passes.push(pass);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "files" => {
                    for raw in lines {
                        let Some(line) = match_tag_line(strip_comment(raw), &tags) else {
                            continue;
                        };
                        let candidate = PathBuf::from(line);
                        let resolved = if candidate.is_absolute() {
                            candidate
                        } else {
                            base_dir.join(candidate)
                        };
                        files.push(resolved);
                    }
                }
                _ => {}
            }
        }

        if engines.is_empty() {
            engines.push(SbyEngine::SmtBmc {
                solver: "z3".to_string(),
            });
        }

        configs.push(SbyTaskConfig {
            name: task_name,
            mode,
            depth,
            skip,
            append,
            multiclock,
            expect,
            engines,
            passes,
            compile: CompileOptions {
                files,
                top,
                defines,
                include_dirs: vec![base_dir.to_path_buf()],
                params,
                formal_mode: true,
            },
            inline_files,
        });
    }

    Ok(configs)
}

pub fn run_sby_task(cfg: &SbyTaskConfig, workdir: Option<&Path>) -> Result<SbyTaskResult> {
    let mut compile = cfg.compile.clone();

    // Materialize `[file <name>]` inline files into a temp or workdir folder.
    let mut temp_inline_dir: Option<PathBuf> = None;
    if !cfg.inline_files.is_empty() {
        let dir = if let Some(wd) = workdir {
            let d = wd.join("src");
            fs::create_dir_all(&d)?;
            d
        } else {
            let pid = std::process::id();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let d = std::env::temp_dir().join(format!("veloformal_sby_{}_{pid}_{ts}", cfg.name));
            fs::create_dir_all(&d)?;
            temp_inline_dir = Some(d.clone());
            d
        };
        compile.include_dirs.push(dir.clone());
        for (fname, content) in &cfg.inline_files {
            let fp = dir.join(fname);
            fs::write(&fp, content)?;
            if fname.ends_with(".v") || fname.ends_with(".sv") {
                compile.files.push(fp);
            }
        }
    }

    if compile.files.is_empty() {
        if let Some(d) = temp_inline_dir {
            let _ = fs::remove_dir_all(d);
        }
        bail!("task '{}': no Verilog/SV source files specified", cfg.name);
    }

    let mut module = compile_to_ir(&compile)?;
    if let Some(d) = temp_inline_dir {
        let _ = fs::remove_dir_all(d);
    }

    // Apply requested script passes (`chformal`, `async2sync`, `clk2fflogic`).
    let mut applied_clk2ff = false;
    for pass in &cfg.passes {
        match pass {
            ScriptPass::Chformal { filter, action } => chformal(&mut module, *filter, *action),
            ScriptPass::Async2Sync => async2sync(&mut module),
            ScriptPass::Clk2FfLogic => {
                clk2fflogic(&mut module);
                applied_clk2ff = true;
            }
        }
    }
    if (cfg.multiclock || has_multiple_clocks(&module)) && !applied_clk2ff {
        clk2fflogic(&mut module);
    }
    // Automatically run `async2sync` if any FF has an async reset so synchronous engines see it immediately.
    if module.ffs.iter().any(|f| f.async_reset.is_some()) {
        async2sync(&mut module);
    }

    let mut vcd_files = Vec::new();
    let (status, detail, traces): (String, String, Vec<(String, Vec<vf_smt::solver::TraceFrame>)>) =
        match cfg.mode {
            SbyMode::Bmc => {
                let solver = first_smt_solver(&cfg.engines);
                match run_bmc(
                    &module,
                    &BmcConfig {
                        depth: cfg.depth,
                        solver,
                        skip: cfg.skip,
                        append: cfg.append,
                    },
                )? {
                    BmcOutcome::Pass { depth } => {
                        ("PASS".into(), format!("BMC passed depth {depth}"), vec![])
                    }
                    BmcOutcome::Fail {
                        property,
                        step,
                        trace,
                    } => (
                        "FAIL".into(),
                        format!("assertion `{property}` violated at cycle {step}"),
                        vec![("trace.vcd".into(), trace)],
                    ),
                    BmcOutcome::AssumptionsUnsat { step } => (
                        "FAIL".into(),
                        format!("assumptions unsatisfiable at cycle {step}"),
                        vec![],
                    ),
                    BmcOutcome::Unknown { step } => {
                        ("UNKNOWN".into(), format!("solver unknown at cycle {step}"), vec![])
                    }
                }
            }
            SbyMode::Prove => {
                let outcome = run_prove_portfolio(&module, cfg)?;
                match outcome {
                    ProveOutcome::Proved { k } => {
                        let via = if cfg.engines.contains(&SbyEngine::AbcPdr) && k == 0 {
                            "PROVED (unbounded)".to_string()
                        } else {
                            format!("PROVED (unbounded, k={k})")
                        };
                        ("PASS".into(), via, vec![])
                    }
                    ProveOutcome::Fail {
                        property,
                        step,
                        trace,
                    } => (
                        "FAIL".into(),
                        format!("assertion `{property}` violated at cycle {step}"),
                        vec![("trace.vcd".into(), trace)],
                    ),
                    ProveOutcome::AssumptionsUnsat { step } => (
                        "FAIL".into(),
                        format!("assumptions unsatisfiable at cycle {step}"),
                        vec![],
                    ),
                    ProveOutcome::Unknown { max_k } => (
                        "UNKNOWN".into(),
                        format!("undecided up to k={max_k}"),
                        vec![],
                    ),
                }
            }
            SbyMode::Cover => {
                let solver = first_smt_solver(&cfg.engines);
                let results = run_cover(
                    &module,
                    &CoverConfig {
                        depth: cfg.depth,
                        solver,
                        append: cfg.append,
                    },
                )?;
                let mut all_ok = true;
                let mut out_traces = Vec::new();
                let mut msgs = Vec::new();
                for (i, r) in results.iter().enumerate() {
                    if let Some((step, tr)) = &r.reached {
                        msgs.push(format!("`{}`@{} ", r.name, step));
                        out_traces.push((format!("trace{i}.vcd"), tr.clone()));
                    } else {
                        msgs.push(format!("UNREACHED `{}` ", r.name));
                        all_ok = false;
                    }
                }
                (
                    if all_ok { "PASS" } else { "FAIL" }.into(),
                    msgs.join("; "),
                    out_traces,
                )
            }
            SbyMode::Live => {
                let solver = first_smt_solver(&cfg.engines);
                match run_live(
                    &module,
                    &LiveConfig {
                        depth: cfg.depth,
                        solver,
                    },
                )? {
                    LiveOutcome::Pass { depth } => (
                        "PASS".into(),
                        format!("no fair starving lasso up to depth {depth}"),
                        vec![],
                    ),
                    LiveOutcome::Fail {
                        property,
                        loop_start,
                        loop_end,
                        trace,
                    } => (
                        "FAIL".into(),
                        format!(
                            "liveness `{property}` starved on fair lasso [{loop_start}..{loop_end}]"
                        ),
                        vec![("trace.vcd".into(), trace)],
                    ),
                    LiveOutcome::Unknown { step } => {
                        ("UNKNOWN".into(), format!("solver unknown at step {step}"), vec![])
                    }
                }
            }
        };

    // Write SBY-compatible workdir artifacts if requested (`status` + `engine_0/*.vcd`).
    if let Some(wd) = workdir {
        fs::create_dir_all(wd)?;
        fs::write(wd.join("status"), format!("{status}\n"))?;
        if !traces.is_empty() {
            let eng_dir = wd.join("engine_0");
            fs::create_dir_all(&eng_dir)?;
            for (fname, trace) in &traces {
                let p = eng_dir.join(fname);
                fs::write(&p, vcd::write_vcd(&module, trace))?;
                vcd_files.push(p);
            }
        }
    }

    let expected_ok = cfg.expect.contains(&status);
    Ok(SbyTaskResult {
        task: cfg.name.clone(),
        status,
        expected_ok,
        detail,
        vcd_files,
    })
}

/// Run all configured prove engines (k-induction and/or ABC PDR) concurrently; first
/// definitive `Proved` or `Fail` wins (SymbiYosys portfolio behavior).
fn run_prove_portfolio(module: &Module, cfg: &SbyTaskConfig) -> Result<ProveOutcome> {
    if cfg.engines.len() == 1 {
        return match &cfg.engines[0] {
            SbyEngine::SmtBmc { solver } => run_kinduction(
                module,
                &ProveConfig {
                    max_k: cfg.depth,
                    solver: solver.clone(),
                },
            ),
            SbyEngine::AbcPdr => run_pdr(module, &PdrConfig::default()),
        };
    }

    let (tx, rx) = mpsc::channel::<Result<ProveOutcome>>();
    let mut handles = Vec::new();
    for eng in &cfg.engines {
        let m_clone = module.clone();
        let eng_clone = eng.clone();
        let depth = cfg.depth;
        let tx_clone = tx.clone();
        handles.push(thread::spawn(move || {
            let res = match eng_clone {
                SbyEngine::SmtBmc { solver } => {
                    run_kinduction(&m_clone, &ProveConfig { max_k: depth, solver })
                }
                SbyEngine::AbcPdr => run_pdr(&m_clone, &PdrConfig::default()),
            };
            let _ = tx_clone.send(res);
        }));
    }
    drop(tx);

    let mut fallback: Option<ProveOutcome> = None;
    for res in rx {
        match res {
            Ok(out @ (ProveOutcome::Proved { .. } | ProveOutcome::Fail { .. })) => {
                return Ok(out);
            }
            Ok(other) => {
                fallback = Some(other);
            }
            Err(_) => {}
        }
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(fallback.unwrap_or(ProveOutcome::Unknown { max_k: cfg.depth }))
}

fn first_smt_solver(engines: &[SbyEngine]) -> String {
    for e in engines {
        if let SbyEngine::SmtBmc { solver } = e {
            return solver.clone();
        }
    }
    "z3".to_string()
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

fn match_tag_line<'a>(line: &'a str, tags: &HashSet<String>) -> Option<&'a str> {
    if line.is_empty() {
        return None;
    }
    // Check for `tag1 tag2: body` or `~tag: body` conditional prefix (note: avoid colon inside paths like C:\)
    if let Some((prefix, rest)) = line.split_once(':') {
        let p = prefix.trim();
        let valid_prefix = !p.is_empty()
            && !p.contains('/')
            && !p.contains('\\')
            && p.split_whitespace()
                .all(|t| t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '~' || c == '-'));
        // Only treat as tag prefix when tasks/tags exist or prefix starts with `~` or matches a known tag.
        if valid_prefix && (!tags.is_empty() || p.starts_with('~')) {
            let matched = p.split_whitespace().any(|tok| {
                if let Some(neg_tok) = tok.strip_prefix('~') {
                    !tags.contains(neg_tok)
                } else {
                    tags.contains(tok)
                }
            });
            return if matched { Some(rest.trim()) } else { None };
        }
    }
    Some(line)
}

fn parse_chformal(args: &[&str]) -> Option<ScriptPass> {
    let mut filter = None;
    let mut action = None;
    for &a in args {
        match a {
            "-assert" => filter = Some(PropertyKind::Assert),
            "-assume" => filter = Some(PropertyKind::Assume),
            "-cover" => filter = Some(PropertyKind::Cover),
            "-live" => filter = Some(PropertyKind::Live),
            "-fair" => filter = Some(PropertyKind::Fair),
            "-remove" => action = Some(ChformalAction::Remove),
            "-assert2assume" => action = Some(ChformalAction::Assert2Assume),
            "-assume2assert" => action = Some(ChformalAction::Assume2Assert),
            "-cover2assert" => action = Some(ChformalAction::Cover2Assert),
            "-live2fair" => action = Some(ChformalAction::Live2Fair),
            "-fair2live" => action = Some(ChformalAction::Fair2Live),
            _ => {}
        }
    }
    action.map(|a| ScriptPass::Chformal { filter, action: a })
}
