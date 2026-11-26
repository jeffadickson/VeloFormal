use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use vf_apps::compose::{parse_sub_arg, run_compose, ComposeConfig};
use vf_apps::vip::{render_vip_sva, render_vip_vfc, VIP_CATALOG};
use vf_apps::vplan::run_vplan_file;
use vf_dsl::{lower_contract_to_ir, parse_contract, ContractRole};
use vf_engine::btor_engine::{detect_btor_solver, validate_btor2_export};
use vf_slang::{compile_to_ir, CompileOptions};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn vf_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_veloformal"))
}

#[test]
fn test_btor2_word_level_export_validation() {
    let root = repo_root();
    let sv = root.join("examples/24_hierarchical_compose/heavy_alu_leaf.sv");
    let vfc = root.join("examples/24_hierarchical_compose/heavy_alu_leaf.vfc");

    let base = compile_to_ir(&CompileOptions::for_files(vec![sv])).expect("compile SV");
    let contract = parse_contract(&std::fs::read_to_string(vfc).unwrap()).expect("parse vfc");
    let lowered = lower_contract_to_ir(&base, &contract, ContractRole::Provider, &Default::default())
        .expect("lower vfc");

    let (btor_text, summary) = validate_btor2_export(&lowered.module).expect("valid BTOR2 export");
    assert!(btor_text.lines().count() > 10, "expected non-trivial BTOR2 output");
    assert!(summary.nodes > 10, "expected non-trivial BTOR2 node count");
    assert!(
        summary.states >= 5,
        "expected leaf FFs exported as BTOR2 states, got {}",
        summary.states
    );
    assert!(summary.bads >= 1, "expected at least one `bad` assertion node");
    assert!(
        summary.constraints >= 1,
        "expected at least one `constraint` assume node"
    );

    // Detection never panics regardless of whether pono/avr/btormc are installed.
    let _ = detect_btor_solver();
}

#[test]
fn test_hierarchical_compose_proves_and_catches_bad_caller() {
    let root = repo_root();
    let dir = root.join("examples/24_hierarchical_compose");
    let leaf_sv = dir.join("heavy_alu_leaf.sv");
    let leaf_vfc = dir.join("heavy_alu_leaf.vfc");
    let top_sv = dir.join("soc_top.sv");
    let top_vfc = dir.join("soc_top.vfc");

    // 1. Clean SoC composition: proves leaf standalone, blackboxes u_alu, proves soc_top!
    let top_files = vec![leaf_sv.clone(), top_sv.clone()];
    let sub_spec = format!("u_alu={}:heavy_alu_leaf:{}", leaf_sv.display(), leaf_vfc.display());
    let sub = parse_sub_arg(&sub_spec).expect("parse --sub");

    let res = run_compose(&ComposeConfig {
        top_rtl_files: top_files,
        top_module: Some("soc_top".into()),
        top_spec_file: top_vfc.clone(),
        top_params: Default::default(),
        submodules: vec![sub],
        depth: 12,
        solver: "z3".into(),
        enable_pdr: true,
    })
    .expect("run_compose clean");

    assert!(res.composed_proved, "clean hierarchical composition must prove!");
    assert_eq!(res.leaf_results.len(), 1);
    assert!(
        res.top_ffs_after_blackbox < res.top_ffs_before_blackbox,
        "expected blackboxing to reduce top FFs ({} -> {})",
        res.top_ffs_before_blackbox,
        res.top_ffs_after_blackbox
    );
    assert!(res.top_blackboxed_signals > 0);

    // 2. Buggy SoC caller: forgets to clamp req_a/req_b -> Caller role MUST catch precondition failure!
    let bad_sv = dir.join("soc_top_bad_caller.sv");
    let bad_files = vec![leaf_sv.clone(), bad_sv];
    let bad_sub = parse_sub_arg(&sub_spec).expect("parse --sub");

    let bad_res = run_compose(&ComposeConfig {
        top_rtl_files: bad_files,
        top_module: Some("soc_top_bad_caller".into()),
        top_spec_file: top_vfc,
        top_params: Default::default(),
        submodules: vec![bad_sub],
        depth: 12,
        solver: "z3".into(),
        enable_pdr: true,
    })
    .expect("run_compose bad caller");

    assert!(
        !bad_res.composed_proved,
        "buggy caller violating u_alu's require must be REFUTED!"
    );
    assert!(
        bad_res.failure_trace.is_some(),
        "expected counterexample trace for caller precondition violation"
    );
}

#[test]
fn test_all_eight_protocol_vips_parse_and_emit_sva() {
    assert_eq!(VIP_CATALOG.len(), 8, "expected 8 built-in turnkey VIP protocols");

    for vip in VIP_CATALOG {
        let vfc = render_vip_vfc(vip.id, Some("dut_test"))
            .unwrap_or_else(|e| panic!("render_vip_vfc({}) failed: {e}", vip.id));
        let parsed = parse_contract(&vfc)
            .unwrap_or_else(|e| panic!("parse_contract for VIP {} failed: {e}\n{vfc}", vip.id));
        assert_eq!(parsed.target_module, "dut_test");

        let sva = render_vip_sva(vip.id, Some("dut_test"))
            .unwrap_or_else(|e| panic!("render_vip_sva({}) failed: {e}", vip.id));
        assert!(
            sva.contains("bind dut_test"),
            "SVA output for {} should contain `bind dut_test`",
            vip.id
        );
    }

    // Verify CLI `veloformal vip list` and `veloformal vip init`
    let list_out = Command::new(vf_bin())
        .args(["vip", "list"])
        .output()
        .expect("run veloformal vip list");
    assert!(list_out.status.success());
    let stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(stdout.contains("axi4_lite"));
    assert!(stdout.contains("tilelink_ul"));
    assert!(stdout.contains("rvfi"));

    let init_out = Command::new(vf_bin())
        .args(["vip", "init", "axi4_lite", "--module", "my_slave", "--sva"])
        .output()
        .expect("run veloformal vip init");
    assert!(init_out.status.success());
    let sva_out = String::from_utf8_lossy(&init_out.stdout);
    assert!(sva_out.contains("bind my_slave"));
}

#[test]
fn test_vplan_end_to_end_matrix_and_cli() {
    let root = repo_root();
    let vplan_path = root.join("examples/24_hierarchical_compose/soc_vplan.json");

    let report = run_vplan_file(&vplan_path).expect("run_vplan_file");
    assert_eq!(report.total_requirements, 3);
    assert_eq!(
        report.passed_requirements, 3,
        "all 3 vplan requirements should pass: {:#?}",
        report.items
    );
    assert_eq!(report.failed_requirements, 0);
    assert!((report.coverage_pct - 100.0).abs() < 1e-6);

    let md = report.to_markdown();
    assert!(md.contains("REQ-ALU-01"));
    assert!(md.contains("REQ-ALU-02"));
    assert!(md.contains("REQ-SOC-01"));
    assert!(md.contains("100.0%"));

    let html = report.to_html();
    assert!(html.contains("VeloFormal Compositional SoC Sign-Off"));
    assert!(html.contains("REQ-SOC-01"));

    // Verify CLI `veloformal vplan` with --html and --md outputs
    let tmp = std::env::temp_dir().join("vf_vplan_test");
    let _ = std::fs::create_dir_all(&tmp);
    let html_out = tmp.join("vplan.html");
    let md_out = tmp.join("vplan.md");

    let out = Command::new(vf_bin())
        .args([
            "vplan",
            vplan_path.to_str().unwrap(),
            "--format",
            "json",
            "--html",
            html_out.to_str().unwrap(),
            "--md",
            md_out.to_str().unwrap(),
        ])
        .output()
        .expect("run veloformal vplan");
    assert!(
        out.status.success(),
        "veloformal vplan failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json_val: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("valid vplan JSON");
    assert_eq!(json_val["passed_requirements"], 3);
    assert!(html_out.exists());
    assert!(md_out.exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_interactive_html_waveform_and_backtrace_debugger() {
    let root = repo_root();
    let dir = root.join("examples/24_hierarchical_compose");
    let tmp = std::env::temp_dir().join("vf_wave_report_test.html");
    let _ = std::fs::remove_file(&tmp);

    // Run compose on soc_top_bad_caller to generate a real failure trace + interactive HTML debugger!
    let leaf_sv = dir.join("heavy_alu_leaf.sv");
    let leaf_vfc = dir.join("heavy_alu_leaf.vfc");
    let bad_sv = dir.join("soc_top_bad_caller.sv");
    let top_vfc = dir.join("soc_top.vfc");
    let sub_arg = format!("u_alu={}:heavy_alu_leaf:{}", leaf_sv.display(), leaf_vfc.display());

    let out = Command::new(vf_bin())
        .args([
            "compose",
            leaf_sv.to_str().unwrap(),
            bad_sv.to_str().unwrap(),
            "--top",
            "soc_top_bad_caller",
            "--spec",
            top_vfc.to_str().unwrap(),
            "--sub",
            &sub_arg,
            "--report",
            tmp.to_str().unwrap(),
        ])
        .output()
        .expect("run veloformal compose --report");

    assert!(
        !out.status.success(),
        "bad caller compose should exit non-zero"
    );
    let html = std::fs::read_to_string(&tmp).expect("read generated HTML report");
    assert!(html.contains("wdata-"), "HTML must embed waveform JSON");
    assert!(html.contains("setRadix("), "HTML must contain interactive radix switcher");
    assert!(html.contains("setSigFilter("), "HTML must contain interactive signal filter");
    assert!(html.contains("zoomWave("), "HTML must contain interactive zoom controls");
    assert!(html.contains("jumpCycle("), "HTML must contain clickable cycle cursor");
    assert!(html.contains("bt-line"), "HTML must contain clickable COI Root-Cause Backtrace");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_mcp_vip_and_compose_tools() {
    let root = repo_root();
    let dir = root.join("examples/24_hierarchical_compose");
    let mcp_bin = vf_bin().parent().unwrap().join("vf-mcp");
    if !mcp_bin.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "vf-mcp"])
            .current_dir(&root)
            .status()
            .expect("build vf-mcp");
        assert!(status.success());
    }

    let req1 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_vip_template",
            "arguments": {}
        }
    });
    let req2 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "compose_verify",
            "arguments": {
                "top_files": [
                    dir.join("heavy_alu_leaf.sv").to_str().unwrap(),
                    dir.join("soc_top.sv").to_str().unwrap()
                ],
                "top": "soc_top",
                "top_spec": dir.join("soc_top.vfc").to_str().unwrap(),
                "subs": [format!(
                    "u_alu={}:heavy_alu_leaf:{}",
                    dir.join("heavy_alu_leaf.sv").display(),
                    dir.join("heavy_alu_leaf.vfc").display()
                )]
            }
        }
    });

    let mut child = Command::new(&mcp_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn vf-mcp");
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{req1}").unwrap();
        writeln!(stdin, "{req2}").unwrap();
    }
    let output = child.wait_with_output().expect("wait vf-mcp");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "expected 2 JSON-RPC responses, got:\n{stdout}");

    let r1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let t1: serde_json::Value =
        serde_json::from_str(r1["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(t1["vips"].as_array().unwrap().len(), 8);

    let r2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    let t2: serde_json::Value =
        serde_json::from_str(r2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(t2["composed_proved"], true);
    assert!(
        t2["top_ffs_after_blackbox"].as_u64().unwrap()
            < t2["top_ffs_before_blackbox"].as_u64().unwrap()
    );
}
