use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use vf_dsl::{lower_contract_to_ir, parse_contract, ContractRole};
use vf_engine::kind::ProveOutcome;
use vf_engine::portfolio::{run_portfolio, PortfolioConfig};
use vf_ir::passes::{async2sync, clk2fflogic, has_multiple_clocks};
use vf_slang::{compile_to_ir, CompileOptions};

use crate::agent::{run_trust_audit, TrustAuditConfig};
use crate::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use crate::compose::{run_compose, ComposeConfig};

/// Declarative Formal Verification Plan (`vplan.json`) mapping design requirements to formal proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPlan {
    pub project: String,
    #[serde(default)]
    pub version: String,
    pub requirements: Vec<PlanRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequirement {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub owner: String,
    /// Verification kind: `"contract"`, `"audit"`, `"autocheck"`, `"compose"`.
    pub kind: String,
    #[serde(default)]
    pub rtl_files: Vec<PathBuf>,
    #[serde(default)]
    pub top: Option<String>,
    #[serde(default)]
    pub spec_file: Option<PathBuf>,
    #[serde(default)]
    pub compose_config: Option<ComposeConfig>,
    #[serde(default)]
    pub params: HashMap<String, i64>,
    #[serde(default = "default_depth")]
    pub depth: usize,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub auto_fsm: bool,
    #[serde(default)]
    pub min_mutation_score: Option<f64>,
}

fn default_depth() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItemResult {
    pub id: String,
    pub title: String,
    pub owner: String,
    pub kind: String,
    pub passed: bool,
    pub trustworthy: Option<bool>,
    pub mutation_score_pct: Option<f64>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPlanReport {
    pub project: String,
    pub version: String,
    pub total_requirements: usize,
    pub passed_requirements: usize,
    pub failed_requirements: usize,
    pub coverage_pct: f64,
    pub avg_mutation_score_pct: Option<f64>,
    pub items: Vec<PlanItemResult>,
}

impl VerificationPlanReport {
    pub fn all_passed(&self) -> bool {
        self.failed_requirements == 0
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Formal Verification Plan — {}", self.project);
        let _ = writeln!(
            out,
            "\n**Coverage:** {}/{} ({:.1}%) | **Status:** {}\n",
            self.passed_requirements,
            self.total_requirements,
            self.coverage_pct,
            if self.all_passed() { "SIGN-OFF PASS" } else { "INCOMPLETE / FAIL" }
        );
        let _ = writeln!(out, "| Req ID | Title | Kind | Status | Trust Audit | Mutation | Detail |");
        let _ = writeln!(out, "|---|---|---|---|---|---|---|");
        for item in &self.items {
            let st = if item.passed { "PASS" } else { "FAIL" };
            let tr = match item.trustworthy {
                Some(true) => "TRUSTWORTHY",
                Some(false) => "FAILED",
                None => "-",
            };
            let muts = match item.mutation_score_pct {
                Some(v) => format!("{v:.1}%"),
                None => "-".into(),
            };
            let _ = writeln!(
                out,
                "| `{}` | {} | `{}` | **{}** | {} | {} | {} |",
                item.id, item.title, item.kind, st, tr, muts, item.detail
            );
        }
        out
    }

    pub fn to_html(&self) -> String {
        let mut rows = String::new();
        for item in &self.items {
            let cls = if item.passed { "pass" } else { "fail" };
            let badge = if item.passed { "PASS" } else { "FAIL" };
            let tr = match item.trustworthy {
                Some(true) => "TRUSTWORTHY",
                Some(false) => "REJECTED",
                None => "—",
            };
            let muts = match item.mutation_score_pct {
                Some(v) => format!("{v:.1}%"),
                None => "—".into(),
            };
            let _ = writeln!(
                rows,
                "<tr><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td><span class=\"badge {cls}\">{badge}</span></td><td>{tr}</td><td>{muts}</td><td>{}</td></tr>",
                html_esc(&item.id),
                html_esc(&item.title),
                html_esc(&item.kind),
                html_esc(&item.detail)
            );
        }
        format!(
            r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>VeloFormal VPlan — {proj}</title>
<style>
body{{font-family:-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif;background:#0d1117;color:#e6edf3;margin:2rem}}
h1{{margin:0 0 .5rem}} .meta{{color:#8b949e;margin-bottom:1.5rem}}
.cards{{display:flex;gap:1rem;margin-bottom:1.5rem;flex-wrap:wrap}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:8px;padding:1rem 1.4rem;min-width:160px}}
.card .v{{font-size:1.8rem;font-weight:700}} .card .k{{color:#8b949e;font-size:.85rem}}
table{{width:100%;border-collapse:collapse;background:#161b22;border:1px solid #30363d;border-radius:8px;overflow:hidden}}
th,td{{text-align:left;padding:.75rem 1rem;border-bottom:1px solid #21262d}}
th{{background:#1f242c;color:#8b949e;font-size:.8rem;text-transform:uppercase}}
.badge{{padding:.2rem .55rem;border-radius:12px;font-weight:600;font-size:.78rem}}
.pass{{background:#1f4d2e;color:#3fb950}} .fail{{background:#4d1f24;color:#f85149}}
code{{background:#21262d;padding:.15rem .35rem;border-radius:4px}}
</style></head><body>
<h1>Formal Verification Plan: {proj}</h1>
<div class="meta">Version {ver} • VeloFormal Traceable Requirements Sign-Off Matrix</div>
<div class="cards">
  <div class="card"><div class="v">{passed}/{total}</div><div class="k">Requirements Verified</div></div>
  <div class="card"><div class="v">{cov:.1}%</div><div class="k">Requirement Coverage</div></div>
  <div class="card"><div class="v">{mut_avg}</div><div class="k">Avg Mutation Score</div></div>
</div>
<table><thead><tr><th>Req ID</th><th>Title</th><th>Engine</th><th>Status</th><th>Trust Audit</th><th>Mutation</th><th>Detail</th></tr></thead>
<tbody>{rows}</tbody></table></body></html>"#,
            proj = html_esc(&self.project),
            ver = html_esc(&self.version),
            passed = self.passed_requirements,
            total = self.total_requirements,
            cov = self.coverage_pct,
            mut_avg = self
                .avg_mutation_score_pct
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into()),
        )
    }
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn resolve_rel(base_dir: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

/// Load a JSON verification plan from disk and execute every traceable requirement end-to-end.
pub fn run_vplan_file(plan_path: &Path) -> Result<VerificationPlanReport> {
    let text = fs::read_to_string(plan_path)
        .with_context(|| format!("failed to read vplan {}", plan_path.display()))?;
    let plan: VerificationPlan = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse JSON vplan {}", plan_path.display()))?;
    let base_dir = plan_path.parent().unwrap_or_else(|| Path::new("."));
    run_vplan(&plan, base_dir)
}

pub fn run_vplan(plan: &VerificationPlan, base_dir: &Path) -> Result<VerificationPlanReport> {
    let mut items = Vec::with_capacity(plan.requirements.len());
    let mut mut_scores = Vec::new();

    for req in &plan.requirements {
        let rtl_files: Vec<PathBuf> = req
            .rtl_files
            .iter()
            .map(|p| resolve_rel(base_dir, p))
            .collect();
        let spec_file = req.spec_file.as_ref().map(|p| resolve_rel(base_dir, p));

        match req.kind.as_str() {
            "autocheck" => {
                let mut opts = CompileOptions::for_files(rtl_files);
                opts.top = req.top.clone();
                let mut cfg = if req.strict {
                    AutoCheckConfig::strict()
                } else {
                    AutoCheckConfig::default()
                };
                cfg.depth = req.depth;
                cfg.auto_fsm |= req.auto_fsm;
                let (_, findings) = run_autocheck(&opts, &cfg)?;
                let bugs = findings
                    .iter()
                    .filter(|f| matches!(f.status, CheckStatus::Bug { .. }))
                    .count();
                let passed = bugs == 0;
                items.push(PlanItemResult {
                    id: req.id.clone(),
                    title: req.title.clone(),
                    owner: req.owner.clone(),
                    kind: req.kind.clone(),
                    passed,
                    trustworthy: None,
                    mutation_score_pct: None,
                    detail: format!("{} checks ({bugs} bugs)", findings.len()),
                });
            }
            "contract" => {
                let Some(sf) = spec_file else {
                    bail!("vplan requirement '{}' (contract) missing spec_file", req.id);
                };
                let spec_txt = fs::read_to_string(&sf)?;
                let contract = parse_contract(&spec_txt)?;
                let mut opts = CompileOptions::for_files(rtl_files);
                opts.top = req.top.clone().or_else(|| Some(contract.target_module.clone()));
                let mut base = compile_to_ir(&opts)?;
                if has_multiple_clocks(&base) {
                    clk2fflogic(&mut base);
                }
                if base.ffs.iter().any(|f| f.async_reset.is_some()) {
                    async2sync(&mut base);
                }
                let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &req.params)?;
                let pr = run_portfolio(
                    &lowered.module,
                    &PortfolioConfig {
                        depth: req.depth,
                        ..Default::default()
                    },
                )?;
                let passed = matches!(pr.outcome, ProveOutcome::Proved { .. });
                items.push(PlanItemResult {
                    id: req.id.clone(),
                    title: req.title.clone(),
                    owner: req.owner.clone(),
                    kind: req.kind.clone(),
                    passed,
                    trustworthy: None,
                    mutation_score_pct: None,
                    detail: if passed {
                        format!("PROVED ({})", pr.winner)
                    } else {
                        format!("{:?}", pr.outcome)
                    },
                });
            }
            "audit" => {
                let Some(sf) = spec_file else {
                    bail!("vplan requirement '{}' (audit) missing spec_file", req.id);
                };
                let spec_txt = fs::read_to_string(&sf)?;
                let contract = parse_contract(&spec_txt)?;
                let mut opts = CompileOptions::for_files(rtl_files);
                opts.top = req.top.clone().or_else(|| Some(contract.target_module.clone()));
                let mut base = compile_to_ir(&opts)?;
                if has_multiple_clocks(&base) {
                    clk2fflogic(&mut base);
                }
                if base.ffs.iter().any(|f| f.async_reset.is_some()) {
                    async2sync(&mut base);
                }
                let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &req.params)?;
                let audit = run_trust_audit(
                    &lowered.module,
                    &TrustAuditConfig {
                        depth: req.depth,
                        min_mutation_score: req.min_mutation_score.unwrap_or(50.0),
                        ..Default::default()
                    },
                )?;
                mut_scores.push(audit.mutation_score_pct);
                items.push(PlanItemResult {
                    id: req.id.clone(),
                    title: req.title.clone(),
                    owner: req.owner.clone(),
                    kind: req.kind.clone(),
                    passed: audit.trustworthy,
                    trustworthy: Some(audit.trustworthy),
                    mutation_score_pct: Some(audit.mutation_score_pct),
                    detail: format!(
                        "4-gate audit ({:.1}% mutation killed)",
                        audit.mutation_score_pct
                    ),
                });
            }
            "compose" => {
                let Some(mut ccfg) = req.compose_config.clone() else {
                    bail!("vplan requirement '{}' (compose) missing compose_config", req.id);
                };
                ccfg.top_rtl_files = ccfg
                    .top_rtl_files
                    .iter()
                    .map(|p| resolve_rel(base_dir, p))
                    .collect();
                ccfg.top_spec_file = resolve_rel(base_dir, &ccfg.top_spec_file);
                for sub in &mut ccfg.submodules {
                    sub.rtl_files = sub
                        .rtl_files
                        .iter()
                        .map(|p| resolve_rel(base_dir, p))
                        .collect();
                    sub.spec_file = resolve_rel(base_dir, &sub.spec_file);
                }
                let cres = run_compose(&ccfg)?;
                items.push(PlanItemResult {
                    id: req.id.clone(),
                    title: req.title.clone(),
                    owner: req.owner.clone(),
                    kind: req.kind.clone(),
                    passed: cres.composed_proved,
                    trustworthy: None,
                    mutation_score_pct: None,
                    detail: format!(
                        "composed {} leaves (FFs {} -> {})",
                        cres.leaf_results.len(),
                        cres.top_ffs_before_blackbox,
                        cres.top_ffs_after_blackbox
                    ),
                });
            }
            other => bail!("unsupported vplan requirement kind '{other}' in '{}'", req.id),
        }
    }

    let total = items.len();
    let passed = items.iter().filter(|i| i.passed).count();
    let failed = total - passed;
    let coverage_pct = if total == 0 {
        100.0
    } else {
        (passed as f64 / total as f64) * 100.0
    };
    let avg_mutation_score_pct = if mut_scores.is_empty() {
        None
    } else {
        Some(mut_scores.iter().sum::<f64>() / mut_scores.len() as f64)
    };

    Ok(VerificationPlanReport {
        project: plan.project.clone(),
        version: plan.version.clone(),
        total_requirements: total,
        passed_requirements: passed,
        failed_requirements: failed,
        coverage_pct,
        avg_mutation_score_pct,
        items,
    })
}
