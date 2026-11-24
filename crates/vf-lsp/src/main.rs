use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use vf_dsl::{format_contract, lower_contract_to_ir, parse_contract, ContractRole};
use vf_slang::{compile_to_ir, CompileOptions};

fn main() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut shutdown_requested = false;

    while let Some(msg) = read_lsp_message(&mut reader)? {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                let result = json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "documentFormattingProvider": true,
                        "hoverProvider": true,
                        "documentSymbolProvider": true,
                        "codeLensProvider": { "resolveProvider": false }
                    },
                    "serverInfo": {
                        "name": "vf-lsp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": result }))?;
                }
            }
            "initialized" => {}
            "shutdown" => {
                shutdown_requested = true;
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": null }))?;
                }
            }
            "exit" => {
                if shutdown_requested {
                    return Ok(());
                }
                std::process::exit(1);
            }
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    params.pointer("/textDocument/uri").and_then(Value::as_str),
                    params.pointer("/textDocument/text").and_then(Value::as_str),
                ) {
                    docs.insert(uri.to_string(), text.to_string());
                    let diags = diagnose_document(uri, text);
                    publish_diagnostics(&mut writer, uri, diags)?;
                }
            }
            "textDocument/didChange" => {
                if let (Some(uri), Some(text)) = (
                    params.pointer("/textDocument/uri").and_then(Value::as_str),
                    params
                        .pointer("/contentChanges/0/text")
                        .and_then(Value::as_str),
                ) {
                    docs.insert(uri.to_string(), text.to_string());
                    let diags = diagnose_document(uri, text);
                    publish_diagnostics(&mut writer, uri, diags)?;
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    if let Some(text) = docs.get(uri).cloned() {
                        let diags = diagnose_document(uri, &text);
                        publish_diagnostics(&mut writer, uri, diags)?;
                    }
                }
            }
            "textDocument/formatting" => {
                let mut edits = Vec::new();
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    if let Some(text) = docs.get(uri) {
                        if let Ok(contract) = parse_contract(text) {
                            let formatted = format_contract(&contract);
                            let line_count = text.lines().count() + 1;
                            edits.push(json!({
                                "range": {
                                    "start": { "line": 0, "character": 0 },
                                    "end": { "line": line_count, "character": 0 }
                                },
                                "newText": formatted
                            }));
                        }
                    }
                }
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": edits }))?;
                }
            }
            "textDocument/hover" => {
                let result = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .and_then(|uri| docs.get(uri))
                    .and_then(|text| {
                        let line = params.pointer("/position/line")?.as_u64()? as usize;
                        let col = params.pointer("/position/character")?.as_u64()? as usize;
                        let word = word_at(text, line, col)?;
                        let doc = keyword_hover(&word)?;
                        Some(json!({
                            "contents": { "kind": "markdown", "value": doc }
                        }))
                    })
                    .unwrap_or(Value::Null);
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": result }))?;
                }
            }
            "textDocument/documentSymbol" => {
                let mut syms = Vec::new();
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    if let Some(text) = docs.get(uri) {
                        if let Ok(c) = parse_contract(text) {
                            syms.push(make_symbol(&c.name, 5, c.span.line));
                            for r in &c.requires {
                                syms.push(make_symbol(&format!("require {}", r.label), 14, r.span.line));
                            }
                            for inv in &c.invariants {
                                syms.push(make_symbol(&format!("invariant {}", inv.label), 14, inv.span.line));
                            }
                            for op in &c.operations {
                                syms.push(make_symbol(&format!("operation {}", op.name), 12, op.span.line));
                            }
                            for g in &c.ghosts {
                                syms.push(make_symbol(&format!("ghost {}", g.name), 13, g.span.line));
                            }
                            for cov in &c.covers {
                                syms.push(make_symbol(&format!("cover {}", cov.label), 24, cov.span.line));
                            }
                        }
                    }
                }
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": syms }))?;
                }
            }
            "textDocument/codeLens" => {
                let mut lenses = Vec::new();
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    if let Some(text) = docs.get(uri) {
                        if let Ok(c) = parse_contract(text) {
                            let line0 = c.span.line.saturating_sub(1);
                            let range = json!({
                                "start": { "line": line0, "character": 0 },
                                "end": { "line": line0, "character": 10 }
                            });
                            lenses.push(json!({
                                "range": range,
                                "command": {
                                    "title": "▶ Verify Contract (VeloFormal)",
                                    "command": "veloformal.verifyContract",
                                    "arguments": [uri]
                                }
                            }));
                            lenses.push(json!({
                                "range": range,
                                "command": {
                                    "title": "Export SVA bind",
                                    "command": "veloformal.emitSva",
                                    "arguments": [uri]
                                }
                            }));
                        }
                    }
                }
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": lenses }))?;
                }
            }
            _ => {
                if let Some(id) = id {
                    write_lsp_message(&mut writer, &json!({ "jsonrpc": "2.0", "id": id, "result": null }))?;
                }
            }
        }
    }
    Ok(())
}

fn make_symbol(name: &str, kind: u32, line1: usize) -> Value {
    let l = line1.saturating_sub(1);
    let range = json!({
        "start": { "line": l, "character": 0 },
        "end": { "line": l, "character": 40 }
    });
    json!({
        "name": name,
        "kind": kind,
        "range": range,
        "selectionRange": range
    })
}

fn diagnose_document(uri: &str, text: &str) -> Vec<Value> {
    let contract = match parse_contract(text) {
        Ok(c) => c,
        Err(e) => return vec![error_to_diagnostic(&e.to_string(), 1)],
    };

    // Check for `// @rtl <relative_path.sv>` pragma or sibling `<target_module>.sv` for live typechecking!
    if let Some(vfc_path) = uri_to_path(uri) {
        let dir = vfc_path.parent().unwrap_or_else(|| Path::new("."));
        let rtl_path = find_rtl_pragma(text)
            .map(|p| dir.join(p))
            .or_else(|| {
                let candidate = dir.join(format!("{}.sv", contract.target_module));
                candidate.exists().then_some(candidate)
            });
        if let Some(rp) = rtl_path {
            if rp.exists() {
                let mut opts = CompileOptions::for_files(vec![rp]);
                opts.top = Some(contract.target_module.clone());
                if let Ok(ir) = compile_to_ir(&opts) {
                    if let Err(e) =
                        lower_contract_to_ir(&ir, &contract, ContractRole::Provider, &HashMap::new())
                    {
                        return vec![error_to_diagnostic(&e.to_string(), 1)];
                    }
                }
            }
        }
    }
    Vec::new()
}

fn find_rtl_pragma(text: &str) -> Option<String> {
    for line in text.lines().take(10) {
        if let Some(pos) = line.find("@rtl") {
            let rest = line[pos + 4..].trim();
            if let Some(tok) = rest.split_whitespace().next() {
                return Some(tok.to_string());
            }
        }
    }
    None
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://").map(PathBuf::from)
}

fn error_to_diagnostic(msg: &str, severity: u32) -> Value {
    let (line, col) = extract_line_col(msg).unwrap_or((1, 1));
    let l0 = line.saturating_sub(1);
    let c0 = col.saturating_sub(1);
    json!({
        "range": {
            "start": { "line": l0, "character": c0 },
            "end": { "line": l0, "character": c0 + 12 }
        },
        "severity": severity,
        "source": "veloformal",
        "message": msg
    })
}

fn extract_line_col(msg: &str) -> Option<(usize, usize)> {
    let idx = msg.rfind(" at ")?;
    let tail = msg[idx + 4..].trim();
    let tok = tail.split_whitespace().next()?;
    let (l, c) = tok.split_once(':')?;
    let ln: usize = l.trim_end_matches(|ch: char| !ch.is_ascii_digit()).parse().ok()?;
    let cn: usize = c.trim_end_matches(|ch: char| !ch.is_ascii_digit()).parse().ok()?;
    Some((ln, cn))
}

fn word_at(text: &str, line0: usize, col0: usize) -> Option<String> {
    let line = text.lines().nth(line0)?;
    let bytes = line.as_bytes();
    if col0 >= bytes.len() {
        return None;
    }
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = col0;
    while start > 0 && is_id(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col0;
    while end < bytes.len() && is_id(bytes[end]) {
        end += 1;
    }
    if start < end {
        Some(line[start..end].to_string())
    } else {
        None
    }
}

fn keyword_hover(word: &str) -> Option<&'static str> {
    match word {
        "require" => Some("**`require <label>: <expr>;`**\n\nEnvironment precondition. Assumed in `provider` mode; asserted in `caller` mode."),
        "invariant" => Some("**`invariant <label>: <expr>;`**\n\nAlways-on safety guarantee checked at every active (non-reset) cycle."),
        "operation" => Some("**`operation <name> when <trigger> { ensure ... }`**\n\nDefines an accepted transaction; each trigger spawns an independent obligation cancelled on reset."),
        "at_trigger" => Some("**`at_trigger(expr)`**\n\nCaptures `expr` at the cycle when the enclosing `operation` triggered (valid inside `ensure`)."),
        "prev" => Some("**`prev(expr [, cycles])`**\n\nSafe previous-cycle value automatically guarded by post-reset history depth (no time-0 `$past` glitches)."),
        "stable" => Some("**`stable(expr)`**\n\nTrue iff `expr == prev(expr)` (automatically guarded by post-reset history >= 1)."),
        "rose" => Some("**`rose(expr)`**\n\nTrue iff `expr` transitioned `0 -> 1` since the previous active cycle."),
        "fell" => Some("**`fell(expr)`**\n\nTrue iff `expr` transitioned `1 -> 0` since the previous active cycle."),
        "symbolic" => Some("**`symbolic <name>: logic[W];`**\n\nDeclares an unconstrained symbolic constant (`$anyconst`) for data-integrity proofs."),
        "ghost" => Some("**`ghost <name>: logic[W] = init;`**\n\nDeclares an auxiliary verification state register updated in `update { ... }`."),
        "use" => Some("**`use <protocol>(...);`**\n\nInstantiates a built-in formal protocol checker (`handshake`, `apb`, `axi4_lite`, `fifo_scoreboard`)."),
        "xfree" => Some("**`xfree sig1, sig2;`**\n\nRuns 4-state dual-rail X-Check verifying target signals never carry silicon `X`."),
        "secure" => Some("**`secure src -> dst [unless guard];`**\n\nRuns 2-copy self-composition Information Flow Tracking proving non-interference."),
        _ => None,
    }
}

fn publish_diagnostics(writer: &mut impl Write, uri: &str, diags: Vec<Value>) -> Result<()> {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": diags
        }
    });
    write_lsp_message(writer, &notif)
}

fn read_lsp_message(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut content_len: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_len = v.trim().parse().ok();
        }
    }
    let Some(len) = content_len else {
        return Ok(None);
    };
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let val: Value = serde_json::from_slice(&buf)?;
    Ok(Some(val))
}

fn write_lsp_message(writer: &mut impl Write, val: &Value) -> Result<()> {
    let body = serde_json::to_string(val)?;
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}
