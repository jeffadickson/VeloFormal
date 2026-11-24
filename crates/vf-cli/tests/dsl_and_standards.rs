use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use vf_dsl::{emit_sva, format_contract, lower_contract_to_ir, parse_contract, ContractRole};
use vf_slang::{compile_to_ir, CompileOptions};

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
fn dsl_conformance_static_checks_and_idempotent_formatter() {
    // C02: after 0 cycles rejected
    let err = parse_contract(
        "contract T for m { operation go when en { ensure bad after 0 cycles: ok; } }",
    )
    .unwrap_err();
    assert!(err.to_string().contains("after 0 cycles"), "{err}");

    // C03: inverted within bounds rejected
    let err = parse_contract(
        "contract T for m { operation go when en { ensure bad within 5..3 cycles: ok; } }",
    )
    .unwrap_err();
    assert!(err.to_string().contains("inverted bounds"), "{err}");

    // Formatter idempotence on counter.vfc
    let src = fs::read_to_string(repo_root().join("examples/18_vfc_contracts/counter.vfc"))
        .expect("read counter.vfc");
    let c1 = parse_contract(&src).expect("parse 1");
    let f1 = format_contract(&c1);
    let c2 = parse_contract(&f1).expect("parse 2");
    let f2 = format_contract(&c2);
    assert_eq!(f1, f2, "formatter must be idempotent");

    // Compile counter.sv for typechecker tests
    let rtl = compile_to_ir(&CompileOptions::for_files(vec![
        repo_root().join("examples/18_vfc_contracts/counter.sv"),
    ]))
    .expect("compile counter.sv");

    // C01: Parameter constraint `MAX_VAL > 0` violated when MAX_VAL=0
    let mut bad_p = HashMap::new();
    bad_p.insert("MAX_VAL".into(), 0);
    let err = lower_contract_to_ir(&rtl, &c1, ContractRole::Provider, &bad_p).unwrap_err();
    assert!(err.to_string().contains("parameter constraint failed"), "{err}");

    // C04: Unknown signal rejected
    let c_unk = parse_contract("contract T for counter_rtl { invariant x: nonexistent == 1; }").unwrap();
    let err = lower_contract_to_ir(&rtl, &c_unk, ContractRole::Provider, &HashMap::new()).unwrap_err();
    assert!(err.to_string().contains("unknown signal"), "{err}");

    // C05: at_trigger outside ensure rejected
    let c_at = parse_contract("contract T for counter_rtl { invariant x: count == at_trigger(count); }").unwrap();
    let err = lower_contract_to_ir(&rtl, &c_at, ContractRole::Provider, &HashMap::new()).unwrap_err();
    assert!(err.to_string().contains("at_trigger"), "{err}");

    // C06: Mixed signed/unsigned rejected without explicit cast
    let c_sign = parse_contract("contract T for counter_rtl { invariant x: count < 8'sd5; }").unwrap();
    let err = lower_contract_to_ir(&rtl, &c_sign, ContractRole::Provider, &HashMap::new()).unwrap_err();
    assert!(err.to_string().contains("mixed signed and unsigned"), "{err}");

    // C07: Width mismatch rejected without explicit resize
    let c_wid = parse_contract("contract T for counter_rtl { invariant x: count == 4'd3; }").unwrap();
    let err = lower_contract_to_ir(&rtl, &c_wid, ContractRole::Provider, &HashMap::new()).unwrap_err();
    assert!(err.to_string().contains("width mismatch"), "{err}");
}

#[test]
fn dsl_contract_counter_prove_cover_vacuity_and_emit_sva_roundtrip() {
    let tmp_dir = std::env::temp_dir().join("vf_dsl_test_counter");
    let _ = fs::create_dir_all(&tmp_dir);
    let html_path = tmp_dir.join("counter_report.html");
    let sva_path = tmp_dir.join("counter_sva.sv");

    // 1. Run `veloformal contract` with vacuity & HTML dashboard report
    let (code, out) = run_vf(&[
        "contract",
        "examples/18_vfc_contracts/counter.sv",
        "--spec",
        "examples/18_vfc_contracts/counter.vfc",
        "-G",
        "MAX_VAL=5",
        "--vacuity",
        "--report",
        html_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "counter contract failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");
    assert!(out.contains("NON-VACUOUS"), "{out}");

    let html = fs::read_to_string(&html_path).expect("read HTML report");
    assert!(html.contains("VeloFormal Formal Sign-Off Report"), "{html}");
    assert!(html.contains("<svg class=\"wave\""), "expected embedded SVG waveform for cover witness");

    // 2. Round-trip `emit-sva` -> compile emitted IEEE 1800 SVA `bind` file + RTL through Slang -> PROVE!
    let (code, out) = run_vf(&[
        "emit-sva",
        "examples/18_vfc_contracts/counter.vfc",
        "--rtl",
        "examples/18_vfc_contracts/counter.sv",
        "-o",
        sva_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "emit-sva failed:\n{out}");
    let sva_text = fs::read_to_string(&sva_path).unwrap();
    assert!(sva_text.contains("bind counter_rtl CounterSafety_sva"), "{sva_text}");

    let (code, out) = run_vf(&[
        "prove",
        "examples/18_vfc_contracts/counter.sv",
        sva_path.to_str().unwrap(),
        "--top",
        "counter_rtl",
        "-G",
        "MAX_VAL=5",
    ]);
    assert_eq!(code, 0, "emitted SVA proof failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
}

#[test]
fn dsl_contract_fifo_scoreboard_and_key_vault_secure_xfree() {
    // 1. Non-invasive FIFO contract using `use fifo_scoreboard(...)`
    let (code, out) = run_vf(&[
        "contract",
        "examples/18_vfc_contracts/fifo_rtl.sv",
        "--spec",
        "examples/18_vfc_contracts/fifo_contract.vfc",
        "--depth",
        "8",
    ]);
    assert_eq!(code, 0, "fifo contract failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");

    // 2. Key vault security + xfree contract
    let (code, out) = run_vf(&[
        "contract",
        "examples/13_secure_key_vault/key_vault.sv",
        "--spec",
        "examples/18_vfc_contracts/key_vault.vfc",
    ]);
    assert_eq!(code, 0, "key_vault contract failed:\n{out}");
    assert!(out.contains("X-FREE"), "{out}");
    assert!(out.contains("SECURE"), "{out}");
}

#[test]
fn dsl_temporal_within_until_and_ghost_update_contract() {
    let tmp = std::env::temp_dir().join("vf_dsl_temporal_test.vfc");
    let vfc = r#"
contract PulseCheck for pulse_stretcher {
    clock posedge clk;
    reset when rst;

    ghost launches: logic[4] = 0;
    update {
        if (!busy && trig && launches < 15) {
            launches <= launches + 1;
        }
    }

    invariant launches_bounded: launches <= 15;

    operation fire when !busy && trig {
        ensure busy_next after 1 cycles: busy;
        ensure done_in_window within 1..3 cycles: done;
        ensure busy_until_done until done: busy;
    }

    cover reaches_done: done;
}
"#;
    fs::write(&tmp, vfc).unwrap();
    let (code, out) = run_vf(&[
        "contract",
        "examples/19_industry_standards/sva_bind_and_rep.sv",
        "--top",
        "pulse_stretcher",
        "--spec",
        tmp.to_str().unwrap(),
        "--depth",
        "10",
    ]);
    assert_eq!(code, 0, "temporal/ghost contract failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");

    // Also verify emit_sva runs cleanly on complex contract
    let c = parse_contract(vfc).unwrap();
    let sv = emit_sva(&c, None);
    assert!(sv.contains("until"), "{sv}");
    assert!(sv.contains("##[1:3]"), "{sv}");
}

#[test]
fn industry_sva_bind_repetition_and_until_proves() {
    let (code, out) = run_vf(&[
        "prove",
        "examples/19_industry_standards/sva_bind_and_rep.sv",
        "--top",
        "pulse_stretcher",
        "--engine",
        "auto",
        "--depth",
        "10",
    ]);
    assert_eq!(code, 0, "IEEE 1800 SVA bind + [*3] + until failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
}

#[test]
fn industry_systemrdl_and_ipxact_regcheck_pass() {
    // 1. SystemRDL 2.0 (.rdl)
    let (code, out) = run_vf(&[
        "regcheck",
        "examples/15_regcheck_uart_csr/uart_csr.sv",
        "--spec",
        "examples/19_industry_standards/uart_csr.rdl",
    ]);
    assert_eq!(code, 0, "SystemRDL regcheck failed:\n{out}");
    assert!(out.contains("PASS [CTRL]"), "{out}");
    assert!(out.contains("PASS [IRQ_STATUS] access_W1C"), "{out}");
    assert!(out.contains("PASS [IRQ_ENABLE] access_W1S"), "{out}");
    assert!(out.contains("PASS [TX_KICK] access_WO"), "{out}");

    // 2. IEEE 1685 IP-XACT (.xml)
    let (code, out) = run_vf(&[
        "regcheck",
        "examples/15_regcheck_uart_csr/uart_csr.sv",
        "--spec",
        "examples/19_industry_standards/uart_csr.xml",
    ]);
    assert_eq!(code, 0, "IP-XACT regcheck failed:\n{out}");
    assert!(out.contains("PASS [CTRL]"), "{out}");
    assert!(out.contains("PASS [VERSION] access_RO"), "{out}");
    assert!(out.contains("PASS [IRQ_STATUS] access_W1C"), "{out}");
}

#[test]
fn industry_sdc_waives_cdc_false_paths() {
    let (code, out) = run_vf(&[
        "cdc",
        "examples/16_cdc_async_bridge/async_bridge_buggy.sv",
        "--sdc",
        "examples/19_industry_standards/async_bridge.sdc",
    ]);
    assert_eq!(code, 0, "SDC-waived CDC expected exit 0:\n{out}");
    assert!(out.contains("SAFE  [sdc_waived_cdc]"), "{out}");
}

#[test]
fn industry_questa_tcl_runner_and_sarif_junit_json_html_reports() {
    let tmp_html = std::env::temp_dir().join("vf_tcl_signoff.html");

    // SARIF 2.1 output + HTML dashboard
    let (code, out) = run_vf(&[
        "tcl",
        "examples/19_industry_standards/questa_signoff.do",
        "--format",
        "sarif",
        "--report",
        tmp_html.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "Questa Tcl runner failed:\n{out}");
    let sarif: serde_json::Value = serde_json::from_str(&out).expect("valid SARIF JSON");
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "VeloFormal");

    let html = fs::read_to_string(&tmp_html).expect("read Tcl HTML report");
    assert!(html.contains("VeloFormal Formal Sign-Off Report"));

    // JUnit XML output
    let (code, out) = run_vf(&[
        "tcl",
        "examples/19_industry_standards/questa_signoff.do",
        "--format",
        "junit",
    ]);
    assert_eq!(code, 0, "Questa Tcl JUnit failed:\n{out}");
    assert!(out.contains("<testsuite"), "{out}");
    assert!(out.contains("<testcase"), "{out}");
}

#[test]
fn example_22_three_tier_dsl_vs_sva_showcase() {
    // Tier 1: Ready/Valid Handshake Pipe (.vfc + vacuity AND traditional SVA bind)
    let (c1, o1) = run_vf(&[
        "contract",
        "examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.sv",
        "--spec",
        "examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.vfc",
        "--vacuity",
    ]);
    assert_eq!(c1, 0, "Tier 1 .vfc failed:\n{o1}");
    assert!(o1.contains("PROVED") && o1.contains("NON-VACUOUS"), "{o1}");

    let (c1s, o1s) = run_vf(&[
        "prove",
        "examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.sv",
        "examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe_sva.sv",
        "--top",
        "handshake_pipe",
    ]);
    assert_eq!(c1s, 0, "Tier 1 SVA failed:\n{o1s}");
    assert!(o1s.contains("PROVED"), "{o1s}");

    // Tier 2: Synchronous Packet FIFO (.vfc + vacuity AND traditional SVA bind scoreboard)
    let (c2, o2) = run_vf(&[
        "contract",
        "examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.sv",
        "--spec",
        "examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.vfc",
        "--engine",
        "pdr",
        "--vacuity",
    ]);
    assert_eq!(c2, 0, "Tier 2 .vfc failed:\n{o2}");
    assert!(o2.contains("PROVED") && o2.contains("NON-VACUOUS"), "{o2}");

    let (c2s, o2s) = run_vf(&[
        "prove",
        "examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.sv",
        "examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo_sva.sv",
        "--top",
        "packet_fifo",
        "--engine",
        "pdr",
    ]);
    assert_eq!(c2s, 0, "Tier 2 SVA failed:\n{o2s}");
    assert!(o2s.contains("PROVED"), "{o2s}");

    // Tier 3: Secure Crypto Accelerator (.vfc 5-domain unified proof + vacuity + X-FREE + SECURE AND traditional SVA bind)
    let (c3, o3) = run_vf(&[
        "contract",
        "examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv",
        "--spec",
        "examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.vfc",
        "--vacuity",
    ]);
    assert_eq!(c3, 0, "Tier 3 .vfc failed:\n{o3}");
    assert!(
        o3.contains("PROVED")
            && o3.contains("NON-VACUOUS")
            && o3.contains("X-FREE")
            && o3.contains("SECURE"),
        "{o3}"
    );

    let (c3s, o3s) = run_vf(&[
        "prove",
        "examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv",
        "examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine_sva.sv",
        "--top",
        "crypto_engine",
    ]);
    assert_eq!(c3s, 0, "Tier 3 SVA failed:\n{o3s}");
    assert!(o3s.contains("PROVED"), "{o3s}");
}
