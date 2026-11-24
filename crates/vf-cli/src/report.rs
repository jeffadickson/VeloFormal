use serde::Serialize;
use std::fmt::Write;
use vf_smt::solver::TraceFrame;

#[derive(Debug, Clone, Serialize)]
pub struct ReportItem {
    pub name: String,
    pub category: String,
    /// `"PROVED"`, `"PASS"`, `"FAIL"`, `"VACUOUS"`, `"REACHED"`, `"UNREACHABLE"`, `"WARN"`, `"UNKNOWN"`
    pub status: String,
    pub engine: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip)]
    pub trace: Option<Vec<TraceFrame>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignOffReport {
    pub title: String,
    pub module_name: String,
    pub items: Vec<ReportItem>,
}

impl SignOffReport {
    pub fn new(title: impl Into<String>, module_name: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            module_name: module_name.into(),
            items: Vec::new(),
        }
    }

    pub fn has_failures(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.status.as_str(), "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" | "VACUOUS"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn to_junit_xml(&self) -> String {
        let failures = self
            .items
            .iter()
            .filter(|i| matches!(i.status.as_str(), "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" | "VACUOUS"))
            .count();
        let mut out = String::new();
        let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        let _ = writeln!(
            out,
            r#"<testsuite name="{}" tests="{}" failures="{}">"#,
            xml_escape(&self.title),
            self.items.len(),
            failures
        );
        for item in &self.items {
            let _ = write!(
                out,
                r#"  <testcase classname="{}" name="{}">"#,
                xml_escape(&item.category),
                xml_escape(&item.name)
            );
            if matches!(
                item.status.as_str(),
                "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" | "VACUOUS"
            ) {
                let _ = write!(
                    out,
                    r#"<failure message="{}: {}">{}</failure>"#,
                    xml_escape(&item.status),
                    xml_escape(&item.detail),
                    xml_escape(&item.detail)
                );
            }
            out.push_str("</testcase>\n");
        }
        out.push_str("</testsuite>\n");
        out
    }

    pub fn to_sarif(&self) -> String {
        let mut results = Vec::new();
        for item in &self.items {
            let level = match item.status.as_str() {
                "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" => "error",
                "VACUOUS" | "WARN" | "UNKNOWN" => "warning",
                _ => "note",
            };
            let mut loc_arr = Vec::new();
            if let Some(src) = &item.source {
                let parts: Vec<&str> = src.split(':').collect();
                let uri = parts.first().copied().unwrap_or("design.sv");
                let line: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                loc_arr.push(serde_json::json!({
                    "physicalLocation": {
                        "artifactLocation": { "uri": uri },
                        "region": { "startLine": line.max(1) }
                    }
                }));
            }
            results.push(serde_json::json!({
                "ruleId": format!("{}/{}", item.category, item.name),
                "level": level,
                "message": { "text": format!("[{}] {}: {}", item.status, item.name, item.detail) },
                "locations": loc_arr
            }));
        }
        let sarif = serde_json::json!({
            "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
            "version": "2.1.0",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "VeloFormal",
                        "version": env!("CARGO_PKG_VERSION"),
                        "informationUri": "https://github.com/jeffadickson/VeloFormal"
                    }
                },
                "results": results
            }]
        });
        serde_json::to_string_pretty(&sarif).unwrap_or_else(|_| "{}".to_string())
    }

    /// Generate a self-contained Interactive HTML Sign-Off Dashboard with embedded SVG
    /// waveform viewer for every counterexample and cover witness trace.
    pub fn to_html(&self) -> String {
        let total = self.items.len();
        let passed = self
            .items
            .iter()
            .filter(|i| matches!(i.status.as_str(), "PROVED" | "PASS" | "SAFE" | "SECURE" | "X-FREE" | "REACHED" | "NON-VACUOUS"))
            .count();
        let failed = self
            .items
            .iter()
            .filter(|i| matches!(i.status.as_str(), "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" | "VACUOUS"))
            .count();
        let other = total.saturating_sub(passed + failed);

        let mut rows = String::new();
        for (idx, item) in self.items.iter().enumerate() {
            let badge_cls = match item.status.as_str() {
                "PROVED" | "PASS" | "SAFE" | "SECURE" | "X-FREE" | "REACHED" | "NON-VACUOUS" => "ok",
                "FAIL" | "LEAK" | "X-LEAK" | "BUG" | "ERROR" | "VACUOUS" => "fail",
                _ => "warn",
            };
            let wave_html = if let Some(trace) = &item.trace {
                render_waveform_svg(trace, idx)
            } else {
                String::new()
            };
            let toggle = if item.trace.is_some() {
                format!(r#"<button class="wbtn" onclick="toggleWave({idx})">Waveform</button>"#)
            } else {
                String::new()
            };
            let _ = writeln!(
                rows,
                r#"<tr data-status="{badge_cls}">
  <td><span class="badge {badge_cls}">{}</span></td>
  <td class="mono">{}</td>
  <td>{}</td>
  <td class="mono">{}</td>
  <td>{} {toggle}</td>
</tr>"#,
                html_escape(&item.status),
                html_escape(&item.name),
                html_escape(&item.category),
                html_escape(&item.engine),
                html_escape(&item.detail),
            );
            if !wave_html.is_empty() {
                let _ = writeln!(
                    rows,
                    r#"<tr id="wave-{idx}" class="wave-row" style="display:none"><td colspan="5">{wave_html}</td></tr>"#
                );
            }
        }

        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>VeloFormal Sign-Off Report — {title}</title>
<style>
  :root {{ --bg:#0d1117; --panel:#161b22; --border:#30363d; --fg:#e6edf3; --muted:#8b949e; --ok:#2ea043; --fail:#f85149; --warn:#d29922; --accent:#58a6ff; }}
  * {{ box-sizing:border-box; }}
  body {{ margin:0; padding:24px; background:var(--bg); color:var(--fg); font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif; }}
  .wrap {{ max-width:1200px; margin:0 auto; }}
  header {{ display:flex; justify-content:space-between; align-items:baseline; border-bottom:1px solid var(--border); padding-bottom:14px; margin-bottom:20px; }}
  h1 {{ margin:0; font-size:22px; }}
  .sub {{ color:var(--muted); font-size:13px; }}
  .kpis {{ display:grid; grid-template-columns:repeat(4,1fr); gap:12px; margin-bottom:20px; }}
  .kpi {{ background:var(--panel); border:1px solid var(--border); border-radius:8px; padding:14px 16px; }}
  .kpi .lbl {{ color:var(--muted); font-size:12px; text-transform:uppercase; letter-spacing:0.05em; }}
  .kpi .val {{ font-size:28px; font-weight:700; margin-top:4px; }}
  .kpi.ok .val {{ color:var(--ok); }}
  .kpi.fail .val {{ color:var(--fail); }}
  .kpi.warn .val {{ color:var(--warn); }}
  .filters {{ display:flex; gap:8px; margin-bottom:12px; }}
  .filters button {{ background:var(--panel); color:var(--fg); border:1px solid var(--border); border-radius:6px; padding:6px 12px; cursor:pointer; font-size:13px; }}
  .filters button.active {{ border-color:var(--accent); color:var(--accent); }}
  table {{ width:100%; border-collapse:collapse; background:var(--panel); border:1px solid var(--border); border-radius:8px; overflow:hidden; }}
  th, td {{ text-align:left; padding:10px 12px; border-bottom:1px solid var(--border); font-size:13px; vertical-align:top; }}
  th {{ color:var(--muted); font-weight:600; background:#1c2128; }}
  .mono {{ font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; font-size:12px; }}
  .badge {{ display:inline-block; padding:2px 8px; border-radius:999px; font-weight:700; font-size:11px; }}
  .badge.ok {{ background:rgba(46,160,67,0.2); color:#3fb950; border:1px solid rgba(46,160,67,0.4); }}
  .badge.fail {{ background:rgba(248,81,73,0.2); color:#ff7b72; border:1px solid rgba(248,81,73,0.4); }}
  .badge.warn {{ background:rgba(210,153,34,0.2); color:#e3b341; border:1px solid rgba(210,153,34,0.4); }}
  .wbtn {{ margin-left:8px; background:#21262d; color:var(--accent); border:1px solid var(--border); border-radius:4px; padding:2px 8px; cursor:pointer; font-size:11px; }}
  .wave-row td {{ background:#0a0e14; padding:14px; overflow-x:auto; }}
  svg.wave {{ display:block; }}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <div>
      <h1>VeloFormal Formal Sign-Off Report</h1>
      <div class="sub">{title} — Module: <span class="mono">{mod_name}</span></div>
    </div>
    <div class="sub">Engine: VeloFormal v{ver} (Slang + SMT/PDR)</div>
  </header>
  <div class="kpis">
    <div class="kpi"><div class="lbl">Total Checks</div><div class="val">{total}</div></div>
    <div class="kpi ok"><div class="lbl">Proved / Safe</div><div class="val">{passed}</div></div>
    <div class="kpi fail"><div class="lbl">Failures / Bugs</div><div class="val">{failed}</div></div>
    <div class="kpi warn"><div class="lbl">Warnings / Other</div><div class="val">{other}</div></div>
  </div>
  <div class="filters">
    <button class="active" onclick="filterRows('all',this)">All ({total})</button>
    <button onclick="filterRows('ok',this)">Passed ({passed})</button>
    <button onclick="filterRows('fail',this)">Failed ({failed})</button>
    <button onclick="filterRows('warn',this)">Other ({other})</button>
  </div>
  <table>
    <thead><tr><th>Status</th><th>Property / Check</th><th>Category</th><th>Engine</th><th>Details</th></tr></thead>
    <tbody>
{rows}
    </tbody>
  </table>
</div>
<script>
function toggleWave(id) {{
  var r = document.getElementById('wave-' + id);
  if (r) r.style.display = (r.style.display === 'none') ? 'table-row' : 'none';
}}
function filterRows(kind, btn) {{
  document.querySelectorAll('.filters button').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  document.querySelectorAll('tbody tr[data-status]').forEach(tr => {{
    tr.style.display = (kind === 'all' || tr.dataset.status === kind) ? '' : 'none';
  }});
}}
</script>
</body>
</html>
"#,
            title = html_escape(&self.title),
            mod_name = html_escape(&self.module_name),
            ver = env!("CARGO_PKG_VERSION"),
        )
    }
}

fn render_waveform_svg(trace: &[TraceFrame], _id: usize) -> String {
    if trace.is_empty() {
        return String::new();
    }
    let sigs: Vec<&str> = trace[0]
        .values
        .iter()
        .filter(|(n, _)| !n.starts_with('$'))
        .map(|(n, _)| n.as_str())
        .take(24)
        .collect();
    if sigs.is_empty() {
        return String::new();
    }
    let steps = trace.len();
    let label_w = 170usize;
    let cell_w = 64usize;
    let row_h = 28usize;
    let top_h = 24usize;
    let width = label_w + steps * cell_w + 16;
    let height = top_h + sigs.len() * row_h + 8;

    let mut svg = format!(
        r#"<svg class="wave" width="{width}" height="{height}" viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg">"#
    );
    // Cycle header grid
    for t in 0..steps {
        let x = label_w + t * cell_w;
        let _ = write!(
            svg,
            r##"<line x1="{x}" y1="{top_h}" x2="{x}" y2="{height}" stroke="#21262d" stroke-width="1"/>"##
        );
        let cx = x + cell_w / 2;
        let _ = write!(
            svg,
            r##"<text x="{cx}" y="16" fill="#8b949e" font-family="monospace" font-size="11" text-anchor="middle">t={t}</text>"##
        );
    }

    for (r, sig) in sigs.iter().enumerate() {
        let y0 = top_h + r * row_h;
        let y_hi = y0 + 5;
        let y_lo = y0 + row_h - 5;
        let y_mid = y0 + row_h / 2 + 4;
        let _ = write!(
            svg,
            r##"<text x="8" y="{y_mid}" fill="#c9d1d9" font-family="monospace" font-size="12">{}</text>"##,
            html_escape(sig)
        );

        let vals: Vec<&vf_ir::ConstBits> = trace
            .iter()
            .filter_map(|f| f.values.iter().find(|(n, _)| n == sig).map(|(_, v)| v))
            .collect();
        if vals.is_empty() {
            continue;
        }
        if vals[0].width == 1 {
            // 1-bit digital waveform line
            let mut d = String::new();
            for (t, v) in vals.iter().enumerate() {
                let x0 = label_w + t * cell_w;
                let x1 = x0 + cell_w;
                let y = if v.is_zero() { y_lo } else { y_hi };
                if t == 0 {
                    let _ = write!(d, "M {x0} {y} L {x1} {y}");
                } else {
                    let _ = write!(d, " L {x0} {y} L {x1} {y}");
                }
            }
            let _ = write!(
                svg,
                r##"<path d="{d}" fill="none" stroke="#3fb950" stroke-width="2"/>"##
            );
        } else {
            // Multi-bit bus boxes with hex values
            for (t, v) in vals.iter().enumerate() {
                let x0 = label_w + t * cell_w + 2;
                let w = cell_w - 4;
                let h = row_h - 10;
                let hex = match v.to_u64() {
                    Some(n) => format!("0x{n:X}"),
                    None => format!("{}b", v.width),
                };
                let cx = x0 + w / 2;
                let _ = write!(
                    svg,
                    r##"<rect x="{x0}" y="{y_hi}" width="{w}" height="{h}" rx="3" fill="#162232" stroke="#58a6ff" stroke-width="1"/>"##
                );
                let _ = write!(
                    svg,
                    r##"<text x="{cx}" y="{y_mid}" fill="#e6edf3" font-family="monospace" font-size="11" text-anchor="middle">{hex}</text>"##
                );
            }
        }
    }
    svg.push_str("</svg>");
    svg
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
