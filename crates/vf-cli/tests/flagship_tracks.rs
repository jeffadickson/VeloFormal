use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use vf_engine::cert::ProofCertificate;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn run_vf(args: &[&str]) -> (i32, String) {
    let bin = env!("CARGO_BIN_EXE_veloformal");
    let out = Command::new(bin)
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("failed to execute veloformal");
    let code = out.status.code().unwrap_or(-1);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (code, combined)
}

#[test]
fn track1_parallel_portfolio_coi_cache_and_case_split() {
    let cache_dir = std::env::temp_dir().join("vf_test_coi_cache");
    let _ = fs::remove_dir_all(&cache_dir);

    // 1. Parallel portfolio races k-ind + ABC PDR on a non-inductive invariant -> PDR wins!
    let (code, out) = run_vf(&[
        "prove",
        "tests/verilog/pdr_non_inductive.sv",
        "--engine",
        "portfolio",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "portfolio failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("winner="), "{out}");

    // 2. Second run with same cache dir must be an instant cache hit!
    let (code, out) = run_vf(&[
        "prove",
        "tests/verilog/pdr_non_inductive.sv",
        "--engine",
        "portfolio",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "cached portfolio failed:\n{out}");
    assert!(out.contains("cache("), "expected cache hit, got:\n{out}");

    // 3. Parallel case-splitting on a 2-bit selector signal!
    let (code, out) = run_vf(&[
        "prove",
        "examples/01_traffic_light_fsm/traffic_light.sv",
        "--case-split",
        "ew_car=0..1",
    ]);
    assert_eq!(code, 0, "case-split failed:\n{out}");
    assert!(out.contains("case_split("), "{out}");
}

#[test]
fn track1_proof_certificate_emit_verify_and_tamper_rejection() {
    let cert_path = std::env::temp_dir().join("vf_test_counter.vfcert.json");
    let _ = fs::remove_file(&cert_path);

    // Emit certificate on proved contract
    let (code, out) = run_vf(&[
        "contract",
        "examples/18_vfc_contracts/counter.sv",
        "--spec",
        "examples/18_vfc_contracts/counter.vfc",
        "-G",
        "MAX_VAL=5",
        "--emit-cert",
        cert_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "emit-cert failed:\n{out}");
    assert!(cert_path.exists(), "certificate file not created");

    // Verify certificate passes cleanly
    let (code, out) = run_vf(&[
        "contract",
        "examples/18_vfc_contracts/counter.sv",
        "--spec",
        "examples/18_vfc_contracts/counter.vfc",
        "-G",
        "MAX_VAL=5",
        "--verify-cert",
        cert_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "verify-cert failed:\n{out}");
    assert!(out.contains("CERT VERIFIED"), "{out}");

    // Tampering with the certificate MUST be rejected!
    let tampered = fs::read_to_string(&cert_path)
        .unwrap()
        .replace("CounterSafety", "TamperedName");
    fs::write(&cert_path, tampered).unwrap();
    let err = ProofCertificate::load_and_verify(&cert_path, None).unwrap_err();
    assert!(
        err.to_string().contains("integrity verification"),
        "expected tamper rejection, got: {err}"
    );
}

#[test]
fn track2_proof_core_and_formal_mutation_testing() {
    // 1. Proof Core: COI coverage + assumption essentiality
    let (code, out) = run_vf(&[
        "proof-core",
        "examples/06_arbiter_liveness/rr_arbiter_4.sv",
    ]);
    assert_eq!(code, 0, "proof-core failed:\n{out}");
    assert!(out.contains("COI Signal Coverage:"), "{out}");

    // 2. Formal Mutation Testing on counter.sv + counter.vfc
    let mut_html = std::env::temp_dir().join("vf_mutate_report.html");
    let (code, out) = run_vf(&[
        "mutate",
        "examples/18_vfc_contracts/counter.sv",
        "--spec",
        "examples/18_vfc_contracts/counter.vfc",
        "-G",
        "MAX_VAL=5",
        "--max-mutants",
        "8",
        "--report",
        mut_html.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "mutate failed:\n{out}");
    assert!(out.contains("Formal Mutation Score:"), "{out}");
    assert!(out.contains("killed"), "{out}");
    assert!(mut_html.exists());
}

#[test]
fn track3_vf_lsp_stdio_jsonrpc_diagnostics_hover_and_formatting() {
    // Build/locate vf-lsp binary from the same target directory as veloformal.
    let vf_bin = PathBuf::from(env!("CARGO_BIN_EXE_veloformal"));
    let lsp_bin = vf_bin.with_file_name("vf-lsp");
    if !lsp_bin.exists() {
        let status = Command::new("cargo")
            .current_dir(repo_root())
            .args(["build", "-p", "vf-lsp"])
            .status()
            .expect("build vf-lsp");
        assert!(status.success());
    }

    let mut child = Command::new(&lsp_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn vf-lsp");
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let send = |w: &mut std::process::ChildStdin, val: serde_json::Value| {
        let body = serde_json::to_string(&val).unwrap();
        write!(w, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        w.flush().unwrap();
    };
    let recv = |r: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut len = 0usize;
        let mut line = String::new();
        loop {
            line.clear();
            r.read_line(&mut line).unwrap();
            let t = line.trim();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.strip_prefix("Content-Length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    };

    // 1. initialize
    send(&mut stdin, serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    let init_resp = recv(&mut reader);
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "vf-lsp");

    // 2. didOpen with broken .vfc -> expect diagnostic!
    send(&mut stdin, serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///tmp/bad.vfc",
                "languageId": "vfc",
                "version": 1,
                "text": "contract Bad for m { operation o when en { ensure x after 0 cycles: ok; } }"
            }
        }
    }));
    let diag = recv(&mut reader);
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    assert!(!diag["params"]["diagnostics"].as_array().unwrap().is_empty());

    // 3. didChange to valid .vfc -> diagnostics clear, hover & formatting work!
    send(&mut stdin, serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": "file:///tmp/bad.vfc", "version": 2 },
            "contentChanges": [{ "text": "contract Good for m { invariant ok: prev(en) => stable(x); }" }]
        }
    }));
    let diag2 = recv(&mut reader);
    assert!(diag2["params"]["diagnostics"].as_array().unwrap().is_empty());

    send(&mut stdin, serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/bad.vfc" },
            "position": { "line": 0, "character": 37 }
        }
    }));
    let hover = recv(&mut reader);
    assert!(hover["result"]["contents"]["value"].as_str().unwrap().contains("prev"));

    send(&mut stdin, serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/formatting",
        "params": { "textDocument": { "uri": "file:///tmp/bad.vfc" } }
    }));
    let fmt = recv(&mut reader);
    assert!(fmt["result"][0]["newText"].as_str().unwrap().contains("contract Good for m"));

    send(&mut stdin, serde_json::json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown", "params": null }));
    let _ = recv(&mut reader);
    send(&mut stdin, serde_json::json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    assert!(child.wait().unwrap().success());
}

#[test]
fn track4_riscv_rv32i_formal_isa_proof_and_expanded_vip() {
    // 1. Formally verify the 2-stage pipelined RV32I RISC-V core against its `.vfc` ISA contract!
    let (code, out) = run_vf(&[
        "contract",
        "examples/20_riscv_rv32i_formal/rv32i_core.sv",
        "--spec",
        "examples/20_riscv_rv32i_formal/rv32i_isa.vfc",
        "--depth",
        "10",
    ]);
    assert_eq!(code, 0, "RV32I RISC-V formal proof failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("X-FREE"), "{out}");
    assert!(out.contains("REACHED"), "{out}");

    // 2. Verify all 4 new Formal VIP checkers elaborate cleanly through Slang!
    for (files, top) in [
        (vec!["vip/vf_handshake.sv", "vip/vf_axis.sv"], "vf_axis"),
        (vec!["vip/vf_wishbone.sv"], "vf_wishbone"),
        (vec!["vip/vf_ahb_lite.sv"], "vf_ahb_lite"),
        (vec!["vip/vf_handshake.sv", "vip/vf_tilelink_ul.sv"], "vf_tilelink_ul"),
    ] {
        let mut args = vec!["dump-ir"];
        args.extend(files);
        args.push("--top");
        args.push(top);
        let (code, out) = run_vf(&args);
        assert_eq!(code, 0, "VIP {top} failed to elaborate:\n{out}");
        assert!(out.contains(&format!("module \\{top}")), "{out}");
    }
}
