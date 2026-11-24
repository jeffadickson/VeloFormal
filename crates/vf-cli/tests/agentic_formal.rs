use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_veloformal"))
}

fn mcp_bin() -> PathBuf {
    let mut p = cli_bin();
    p.set_file_name(if cfg!(windows) { "vf-mcp.exe" } else { "vf-mcp" });
    p
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(cli_bin())
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("run veloformal");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn cli_inspect_json_and_scaffold_roundtrip_verifies() {
    // 1. Machine-readable JSON RTL inspection on AMBA APB3 CSR block.
    let (ok, stdout, stderr) = run_cli(&[
        "inspect",
        "examples/21_questa_parity/apb_csr.sv",
        "--format",
        "json",
    ]);
    assert!(ok, "inspect failed:\n{stdout}\n{stderr}");
    let info: Value = serde_json::from_str(&stdout).expect("valid inspect JSON");
    assert_eq!(info["module_name"], "apb_csr");
    assert_eq!(info["reset"]["name"], "presetn");
    assert_eq!(info["reset"]["active_low"], true);
    let protos = info["detected_protocols"].as_array().unwrap();
    assert!(
        protos.iter().any(|p| p == "apb"),
        "expected APB protocol detection, got: {info}"
    );

    // 2. Auto-scaffold a starter `.vfc` contract from FIFO RTL, then verify it end-to-end!
    let scaffold_path = std::env::temp_dir().join("vf_scaffolded_fifo.vfc");
    let (ok2, stdout2, stderr2) = run_cli(&[
        "scaffold",
        "examples/18_vfc_contracts/fifo_rtl.sv",
        "-o",
        scaffold_path.to_str().unwrap(),
    ]);
    assert!(ok2, "scaffold failed:\n{stdout2}\n{stderr2}");
    let vfc_text = fs::read_to_string(&scaffold_path).expect("read scaffolded .vfc");
    assert!(vfc_text.contains("contract FifoRtlSpec for fifo_rtl"));
    assert!(vfc_text.contains("not_full_and_empty"));

    let (ok3, stdout3, stderr3) = run_cli(&[
        "contract",
        "examples/18_vfc_contracts/fifo_rtl.sv",
        "--spec",
        scaffold_path.to_str().unwrap(),
    ]);
    let _ = fs::remove_file(&scaffold_path);
    assert!(
        ok3,
        "auto-scaffolded .vfc contract should verify cleanly:\n{stdout3}\n{stderr3}"
    );
    assert!(stdout3.contains("PROVED") && stdout3.contains("REACHED"));
}

#[test]
fn cli_trust_audit_passes_strong_spec_and_rejects_cheating_specs() {
    // 1. Strong spec: Counter + counter.vfc -> TRUSTWORTHY (passes all 4 gates!).
    let (ok, stdout, stderr) = run_cli(&[
        "audit",
        "examples/18_vfc_contracts/counter.sv",
        "--spec",
        "examples/18_vfc_contracts/counter.vfc",
        "-G",
        "MAX_VAL=5",
        "--min-mutation-score",
        "50",
        "--format",
        "json",
    ]);
    assert!(ok, "strong contract should pass trust audit:\n{stdout}\n{stderr}");
    let rep: Value = serde_json::from_str(&stdout).expect("valid audit JSON");
    assert_eq!(rep["trustworthy"], true, "expected trustworthy=true: {rep}");
    assert!(rep["mutation_score_pct"].as_f64().unwrap() >= 50.0);

    // 2. Over-constrained "cheating" spec (dead-end assumptions) -> REJECTED!
    let (ok2, stdout2, _stderr2) = run_cli(&[
        "audit",
        "examples/21_questa_parity/overconstrained_fifo.sv",
        "--format",
        "json",
    ]);
    assert!(!ok2, "over-constrained spec must be rejected:\n{stdout2}");
    let rep2: Value = serde_json::from_str(&stdout2).expect("valid audit JSON");
    assert_eq!(rep2["trustworthy"], false);
    assert!(rep2["deadend_status"]
        .as_str()
        .unwrap()
        .contains("DEAD-END"));

    // 3. Vacuous / trivial "cheating" spec (`if (0) assert(...)` or `assert(1)`) -> REJECTED!
    let cheat_sv = std::env::temp_dir().join("vf_cheat_vacuous.sv");
    fs::write(
        &cheat_sv,
        r#"
module cheat_vacuous (input logic clk, input logic rst, input logic en, output logic [3:0] cnt);
    initial cnt = 4'd0;
    always_ff @(posedge clk) begin
        if (rst) cnt <= 4'd0;
        else if (en && cnt < 4'd8) cnt <= cnt + 4'd1;
    end
    // Cheating vacuous assertion: antecedent `cnt == 4'd15` is never reachable!
    a_cheat: assert property (@(posedge clk) (cnt == 4'd15) |-> (cnt == 4'd0));
endmodule
"#,
    )
    .unwrap();
    let (ok3, stdout3, _) = run_cli(&["audit", cheat_sv.to_str().unwrap(), "--format", "json"]);
    let _ = fs::remove_file(&cheat_sv);
    assert!(!ok3, "vacuous cheating spec must be rejected:\n{stdout3}");
    let rep3: Value = serde_json::from_str(&stdout3).expect("valid audit JSON");
    assert_eq!(rep3["trustworthy"], false);
    assert!(
        !rep3["vacuous_properties"].as_array().unwrap().is_empty(),
        "expected vacuous property flagged: {rep3}"
    );
}

#[test]
fn vf_mcp_stdio_jsonrpc_full_protocol_and_tools() {
    let mut child = Command::new(mcp_bin())
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vf-mcp binary (run `cargo build --workspace` first)");

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let mut call = |id: u64, method: &str, params: Value| -> Value {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).expect("valid JSON-RPC response from vf-mcp")
    };

    // 1. MCP initialize handshake.
    let init = call(
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-agent", "version": "1.0" }
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "veloformal-mcp");

    // 2. tools/list: verify all 10 typed formal tools are registered.
    let tools = call(2, "tools/list", json!({}));
    let tool_arr = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tool_arr.len(), 10, "expected 10 MCP formal tools");

    // 3. resources/read: fetch `.vfc` v1.0 quick-reference spec.
    let spec_res = call(
        3,
        "resources/read",
        json!({ "uri": "veloformal://vfc-spec" }),
    );
    assert!(spec_res["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("contract <ContractName>"));

    // 4. prompts/get: fetch `formal-verify-rtl` workflow prompt.
    let prompt_res = call(
        4,
        "prompts/get",
        json!({ "name": "formal-verify-rtl", "arguments": { "rtl_path": "dut.sv" } }),
    );
    assert!(prompt_res["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("audit_trust"));

    // Helper to parse JSON payload returned inside MCP tool text content.
    let parse_tool_json = |resp: &Value| -> Value {
        assert_eq!(resp["result"]["isError"], false, "tool error: {resp}");
        let txt = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(txt).expect("valid JSON inside MCP tool text")
    };

    // 5. inspect_rtl tool.
    let r5 = call(
        5,
        "tools/call",
        json!({
            "name": "inspect_rtl",
            "arguments": { "files": ["examples/21_questa_parity/apb_csr.sv"] }
        }),
    );
    let p5 = parse_tool_json(&r5);
    assert_eq!(p5["module_name"], "apb_csr");

    // 6. verify_contract with inline `contract_text` (failing contract returns compact Markdown trace_table!).
    let r6 = call(
        6,
        "tools/call",
        json!({
            "name": "verify_contract",
            "arguments": {
                "files": ["examples/18_vfc_contracts/counter.sv"],
                "params": ["MAX_VAL=5"],
                "contract_text": "contract BadSpec for counter_rtl { clock posedge clk; reset when rst; invariant too_tight: unsigned(count) <= 2; }"
            }
        }),
    );
    let p6 = parse_tool_json(&r6);
    assert_eq!(p6["status"], "FAIL");
    assert!(
        p6["trace_table"].as_str().unwrap().contains("| cycle |"),
        "expected compact Markdown cycle table for agent: {p6}"
    );

    // 7. check_equiv SLEC tool (LPGC refactor guardrail!).
    let r7 = call(
        7,
        "tools/call",
        json!({
            "name": "check_equiv",
            "arguments": {
                "gold_files": ["examples/21_questa_parity/lpgc_gold.sv"],
                "gate_files": ["examples/21_questa_parity/lpgc_gate.sv"],
                "reset": "rst=1",
                "valid_cond": "out_valid",
                "ignore_outputs": ["cg_active"]
            }
        }),
    );
    let p7 = parse_tool_json(&r7);
    assert_eq!(p7["status"], "EQUIVALENT");

    // 8. check_deadend tool.
    let r8 = call(
        8,
        "tools/call",
        json!({
            "name": "check_deadend",
            "arguments": { "files": ["examples/21_questa_parity/overconstrained_fifo.sv"], "depth": 8 }
        }),
    );
    let p8 = parse_tool_json(&r8);
    assert_eq!(p8["status"], "DEAD_END");
    assert_eq!(p8["step"], 4);

    // 9. audit_trust tool on valid counter contract -> trustworthy == true!
    let r9 = call(
        9,
        "tools/call",
        json!({
            "name": "audit_trust",
            "arguments": {
                "files": ["examples/18_vfc_contracts/counter.sv"],
                "contract_file": "examples/18_vfc_contracts/counter.vfc",
                "params": ["MAX_VAL=5"],
                "min_mutation_score": 50.0
            }
        }),
    );
    let p9 = parse_tool_json(&r9);
    assert_eq!(p9["trustworthy"], true, "expected trustworthy=true: {p9}");

    drop(stdin);
    let _ = child.wait();
}
