use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use vf_slang::CompileOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbyMode {
    Bmc,
    Prove,
    Cover,
}

#[derive(Debug, Clone)]
pub struct SbyConfig {
    pub mode: SbyMode,
    pub depth: usize,
    pub solver: String,
    pub compile: CompileOptions,
}

/// Parse a SymbiYosys `.sby` configuration file into VeloFormal run options.
pub fn parse_sby(path: &Path) -> Result<SbyConfig> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read .sby file {}", path.display()))?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut section = String::new();
    let mut mode = SbyMode::Bmc;
    let mut depth = 20usize;
    let mut solver = "z3".to_string();
    let mut top: Option<String> = None;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut defines: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        match section.as_str() {
            "options" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                match parts.as_slice() {
                    ["mode", "bmc"] => mode = SbyMode::Bmc,
                    ["mode", "prove"] => mode = SbyMode::Prove,
                    ["mode", "cover"] => mode = SbyMode::Cover,
                    ["mode", other] => bail!("unsupported .sby mode '{other}'"),
                    ["depth", n] => {
                        depth = n
                            .parse()
                            .with_context(|| format!("invalid .sby depth '{n}'"))?;
                    }
                    _ => {}
                }
            }
            "engines" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for &tok in &parts {
                    if tok == "z3" || tok == "bitwuzla" || tok == "cvc5" {
                        solver = tok.to_string();
                    }
                }
            }
            "script" => {
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
                    _ => {}
                }
            }
            "files" => {
                let candidate = PathBuf::from(line);
                let resolved = if candidate.is_absolute() {
                    candidate
                } else {
                    base_dir.join(candidate)
                };
                files.push(resolved);
            }
            _ => {}
        }
    }

    if files.is_empty() {
        bail!("no source files listed in [files] section of {}", path.display());
    }

    Ok(SbyConfig {
        mode,
        depth,
        solver,
        compile: CompileOptions {
            files,
            top,
            defines,
            include_dirs: vec![base_dir.to_path_buf()],
            params,
            formal_mode: true,
        },
    })
}
