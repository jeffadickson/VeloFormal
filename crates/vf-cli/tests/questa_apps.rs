use std::path::PathBuf;
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_apps::cdc::{run_cdc_analysis, CdcConfig, CdcSeverity, GrayStatus};
use vf_apps::connectivity::{parse_connectivity_str, run_connectivity, ConnStatus};
use vf_apps::covercheck::{run_covercheck, CoverCheckConfig, CoverageStatus};
use vf_apps::regcheck::{parse_regmap_str, run_regcheck, RegBusConfig, RegCheckStatus};
use vf_apps::secure::{run_secure_check, SecureConfig, SecureOutcome};
use vf_apps::xcheck::{run_xcheck, XCheckConfig, XStatus};
use vf_engine::equiv::{check_slec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_engine::vacuity::{check_vacuity, VacuityConfig, VacuityStatus};
use vf_ir::passes::{async2sync, clk2fflogic, coi_prune, has_multiple_clocks};
use vf_slang::{compile_to_ir, CompileOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/verilog")
        .join(name)
}

fn vip_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vip")
        .join(name)
}

fn compile_prepared(files: Vec<PathBuf>) -> vf_ir::Module {
    let mut m = compile_to_ir(&CompileOptions::for_files(files)).expect("compile");
    if has_multiple_clocks(&m) {
        clk2fflogic(&mut m);
    }
    if m.ffs.iter().any(|f| f.async_reset.is_some()) {
        async2sync(&mut m);
    }
    m
}

#[test]
fn autocheck_catches_oob_divzero_unique_and_fsm_deadlock() {
    let opts = CompileOptions::for_files(vec![fixture("autocheck_bugs.sv")]);
    let (_, findings) = run_autocheck(
        &opts,
        &AutoCheckConfig {
            depth: 12,
            fsm_signals: vec!["fsm_state".into()],
            ..Default::default()
        },
    )
    .expect("autocheck");

    let has_bug = |cat: &str| {
        findings
            .iter()
            .any(|f| f.category == cat && matches!(f.status, CheckStatus::Bug { .. }))
    };
    assert!(
        has_bug("oob_memory"),
        "expected OOB memory bug, got: {findings:#?}"
    );
    assert!(
        has_bug("div_by_zero"),
        "expected div-by-zero bug, got: {findings:#?}"
    );
    assert!(
        has_bug("unique_priority"),
        "expected unique-case violation, got: {findings:#?}"
    );
    assert!(
        has_bug("fsm_deadlock"),
        "expected FSM deadlock bug, got: {findings:#?}"
    );
}

#[test]
fn autocheck_proves_clean_rtl_safe() {
    let opts = CompileOptions::for_files(vec![fixture("autocheck_clean.sv")]);
    let (_, findings) = run_autocheck(
        &opts,
        &AutoCheckConfig {
            depth: 12,
            fsm_signals: vec!["fsm_state".into()],
            ..Default::default()
        },
    )
    .expect("autocheck");

    for f in &findings {
        assert!(
            matches!(f.status, CheckStatus::Safe),
            "expected all checks SAFE on clean RTL, but `{}` ({}) was {:?}",
            f.name,
            f.category,
            f.status
        );
    }
    assert!(
        findings.len() >= 4,
        "expected at least 4 auto-generated checks, got {}",
        findings.len()
    );
}

#[test]
fn covercheck_finds_reachable_and_proves_dead_branch_unreachable() {
    let opts = CompileOptions::for_files(vec![fixture("covercheck_dead.sv")]);
    let (_, items) = run_covercheck(
        &opts,
        &CoverCheckConfig {
            depth: 12,
            ..Default::default()
        },
    )
    .expect("covercheck");

    let reachable = items
        .iter()
        .filter(|i| matches!(i.status, CoverageStatus::Reachable { .. }))
        .count();
    let unreachable = items
        .iter()
        .filter(|i| matches!(i.status, CoverageStatus::Unreachable))
        .count();

    assert!(
        reachable >= 4,
        "expected >=4 reachable branches, got: {items:#?}"
    );
    assert!(
        unreachable >= 1,
        "expected >=1 proved UNREACHABLE dead branch (cnt == 12), got: {items:#?}"
    );
}

#[test]
fn connectivity_proves_valid_routes_and_catches_miswire() {
    let m = compile_prepared(vec![fixture("mux_crossbar.sv")]);
    let csv = "\
src,dst,cond,invert,delay
in_a,out_mux,sel == 0,0,0
in_b,out_mux,sel == 1,0,0
in_c,out_mux,sel == 2,0,0
in_a,out_inv,,1,0
in_b,out_pipe,,0,1
in_a,out_mux,sel == 1,0,0
";
    let rules = parse_connectivity_str(csv).expect("parse csv");
    let results = run_connectivity(&m, &rules, 10, "z3").expect("connectivity");
    assert_eq!(results.len(), 6);
    for r in &results[..5] {
        assert!(
            matches!(r.status, ConnStatus::Proved),
            "expected rule `{}` PROVED, got {:?}",
            r.rule.name,
            r.status
        );
    }
    assert!(
        matches!(results[5].status, ConnStatus::Fail { .. }),
        "expected miswired rule to FAIL, got {:?}",
        results[5].status
    );
}

#[test]
fn secure_check_detects_leak_proves_isolation_and_declassification() {
    let m = compile_prepared(vec![fixture("secure_crypto.sv")]);

    // 1. Without declassification: secret_key -> pub_out MUST report a LEAK!
    let (_, leak) = run_secure_check(
        &m,
        &SecureConfig {
            sources: vec!["secret_key".into()],
            targets: vec!["pub_out".into()],
            declassify: None,
            depth: 10,
            solver: "z3".into(),
            use_pdr: true,
        },
    )
    .expect("secure check leak");
    assert!(
        matches!(leak, SecureOutcome::Leak { .. }),
        "expected Leak without declassification, got {leak:?}"
    );

    // 2. With `--declassify debug_auth`: secret_key -> pub_out MUST be PROVED SECURE!
    let (_, declass) = run_secure_check(
        &m,
        &SecureConfig {
            sources: vec!["secret_key".into()],
            targets: vec!["pub_out".into()],
            declassify: Some("debug_auth".into()),
            depth: 10,
            solver: "z3".into(),
            use_pdr: true,
        },
    )
    .expect("secure check declassify");
    assert!(
        matches!(declass, SecureOutcome::Secure),
        "expected Secure with debug_auth declassification, got {declass:?}"
    );

    // 3. Strictly isolated output: secret_key -> isolated_out MUST be PROVED SECURE!
    let (_, iso) = run_secure_check(
        &m,
        &SecureConfig {
            sources: vec!["secret_key".into()],
            targets: vec!["isolated_out".into()],
            declassify: None,
            depth: 10,
            solver: "z3".into(),
            use_pdr: true,
        },
    )
    .expect("secure check isolated");
    assert!(
        matches!(iso, SecureOutcome::Secure),
        "expected Secure on isolated_out, got {iso:?}"
    );
}

#[test]
fn xcheck_catches_uninit_x_leak_and_proves_controlling_zero_and_init_xfree() {
    let m = compile_prepared(vec![fixture("xcheck_uninit.sv")]);
    let (_, results) = run_xcheck(
        &m,
        &XCheckConfig {
            targets: vec![
                "out_leak".into(),
                "out_masked".into(),
                "out_init".into(),
            ],
            ..Default::default()
        },
    )
    .expect("xcheck");

    let by_name = |n: &str| {
        &results
            .iter()
            .find(|r| r.signal == n)
            .expect("target")
            .status
    };

    assert!(
        matches!(by_name("out_leak"), XStatus::XLeak { .. }),
        "expected XLeak on out_leak, got {:?}",
        by_name("out_leak")
    );
    assert!(
        matches!(by_name("out_masked"), XStatus::XFree),
        "expected XFree on out_masked (0 & X == 0), got {:?}",
        by_name("out_masked")
    );
    assert!(
        matches!(by_name("out_init"), XStatus::XFree),
        "expected XFree on out_init, got {:?}",
        by_name("out_init")
    );
}

#[test]
fn regcheck_verifies_clean_csr_bank_and_catches_ro_and_alias_bugs() {
    let csv = "\
name,addr,width,reset,access,mask
CTRL,0x0,32,0x00000000,RW,0x000000FF
STATUS,0x4,32,0x000000A5,RO,0xFFFFFFFF
IRQ,0x8,32,0x000000FF,W1C,0x000000FF
ENABLE,0xC,32,0x00000000,W1S,0x000000FF
KICK,0xE,32,0x00000000,WO,0xFFFFFFFF
";
    let specs = parse_regmap_str(csv).expect("parse regmap");

    // Clean CSR bank: EVERY check must PASS!
    let clean = compile_prepared(vec![fixture("csr_bank.sv")]);
    let findings = run_regcheck(
        &clean,
        &specs,
        &RegBusConfig {
            depth: 10,
            solver: "z3".into(),
            use_pdr: true,
            ..Default::default()
        },
    )
    .expect("regcheck clean");
    for f in &findings {
        assert!(
            matches!(f.status, RegCheckStatus::Pass),
            "expected PASS for [{}] {}, got {:?}",
            f.register,
            f.check,
            f.status
        );
    }

    // Buggy CSR bank: STATUS RO write and CTRL no_alias must FAIL!
    let buggy_specs = parse_regmap_str(
        "\
name,addr,width,reset,access,mask
CTRL,0x0,32,0x00000000,RW,0x000000FF
STATUS,0x4,32,0x000000A5,RO,0xFFFFFFFF
",
    )
    .unwrap();
    let buggy = compile_prepared(vec![fixture("csr_bank_buggy.sv")]);
    let bad_findings = run_regcheck(
        &buggy,
        &buggy_specs,
        &RegBusConfig {
            depth: 10,
            solver: "z3".into(),
            use_pdr: true,
            ..Default::default()
        },
    )
    .expect("regcheck buggy");

    assert!(
        bad_findings
            .iter()
            .any(|f| f.register == "STATUS" && f.check == "access_RO" && matches!(f.status, RegCheckStatus::Fail { .. })),
        "expected STATUS RO write bug to FAIL, got: {bad_findings:#?}"
    );
    assert!(
        bad_findings
            .iter()
            .any(|f| f.register == "CTRL" && f.check == "no_alias" && matches!(f.status, RegCheckStatus::Fail { .. })),
        "expected CTRL aliasing bug to FAIL, got: {bad_findings:#?}"
    );
}

#[test]
fn cdc_detects_crossings_rdc_and_verifies_gray_code() {
    let raw = compile_to_ir(&CompileOptions::for_files(vec![fixture("cdc_design.sv")]))
        .expect("compile raw");
    let report = run_cdc_analysis(
        &raw,
        &CdcConfig {
            gray_signals: vec!["gray_cnt".into(), "bin_cnt".into()],
            depth: 12,
            solver: "z3".into(),
            use_pdr: true,
        },
    )
    .expect("cdc analysis");

    assert_eq!(report.clock_domains.len(), 2, "expected 2 clock domains");
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.kind == "synchronized_cdc" && f.severity == CdcSeverity::Safe),
        "expected 2-FF synchronized crossing to be Safe, got: {:#?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.kind == "unsynchronized_cdc" && f.severity == CdcSeverity::Error),
        "expected unsynchronized 1-bit CDC error, got: {:#?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.kind == "multi_bit_cdc" && f.severity == CdcSeverity::Error),
        "expected multi-bit CDC error, got: {:#?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.kind == "rdc_crossing" && f.severity == CdcSeverity::Warning),
        "expected RDC crossing warning, got: {:#?}",
        report.findings
    );

    let gray_ok = report
        .gray_results
        .iter()
        .find(|g| g.signal == "gray_cnt")
        .unwrap();
    assert!(
        matches!(gray_ok.status, GrayStatus::Proved),
        "expected gray_cnt PROVED, got {:?}",
        gray_ok.status
    );

    let bin_bad = report
        .gray_results
        .iter()
        .find(|g| g.signal == "bin_cnt")
        .unwrap();
    assert!(
        matches!(bin_bad.status, GrayStatus::Fail { .. }),
        "expected bin_cnt to FAIL Gray-code check, got {:?}",
        bin_bad.status
    );
}

#[test]
fn vacuity_distinguishes_reachable_vs_vacuous_assertions() {
    let m = compile_prepared(vec![fixture("vacuity_test.sv")]);
    let results = check_vacuity(
        &m,
        &VacuityConfig {
            depth: 15,
            solver: "z3".into(),
            use_pdr: true,
        },
    )
    .expect("vacuity");

    assert_eq!(results.len(), 2);
    let reach = results.iter().find(|r| r.property.contains("a_reachable")).unwrap();
    let vac = results.iter().find(|r| r.property.contains("a_vacuous")).unwrap();

    assert!(
        matches!(reach.status, VacuityStatus::NonVacuous { step: 3, .. }),
        "expected a_reachable NonVacuous at cycle 3, got {:?}",
        reach.status
    );
    assert!(
        matches!(vac.status, VacuityStatus::Vacuous),
        "expected a_vacuous proved Vacuous, got {:?}",
        vac.status
    );
}

#[test]
fn slec_proves_pipelined_latency_offset_equivalence_and_coi_prunes() {
    let gold = compile_prepared(vec![fixture("slec_pipe_gold.sv")]);
    let gate = compile_prepared(vec![fixture("slec_pipe_gate.sv")]);

    // Without latency alignment: MUST fail!
    let (_, wrong) = check_slec(
        &gold,
        &gate,
        &SlecConfig {
            latency_gold: 0,
            ..Default::default()
        },
    )
    .expect("slec unaligned");
    assert!(
        matches!(wrong, ProveOutcome::Fail { .. }),
        "expected unaligned pipeline SLEC to FAIL, got {wrong:?}"
    );

    // With latency_gold = 2: MUST be proved EQUIVALENT unboundedly!
    let (mut miter, right) = check_slec(
        &gold,
        &gate,
        &SlecConfig {
            latency_gold: 2,
            engine: "auto".into(),
            ..Default::default()
        },
    )
    .expect("slec aligned");
    assert!(
        matches!(right, ProveOutcome::Proved { .. }),
        "expected latency-aligned SLEC to PROVE, got {right:?}"
    );

    // Verify COI pruning works cleanly on the miter.
    coi_prune(&mut miter);
    assert!(!miter.properties.is_empty());
}

#[test]
fn formal_vip_handshake_and_apb_checkers_prove_compliant_dut() {
    let mut opts = CompileOptions::for_files(vec![
        vip_file("vf_handshake.sv"),
        vip_file("vf_apb.sv"),
        fixture("vip_dut_test.sv"),
    ]);
    opts.top = Some("vip_dut_test".into());
    let m = compile_to_ir(&opts).expect("compile vip dut");

    // Prove all VIP protocol assertions unboundedly (k-induction or PDR)!
    let res = match run_kinduction(
        &m,
        &ProveConfig {
            max_k: 8,
            solver: "z3".into(),
        },
    )
    .expect("kind")
    {
        ProveOutcome::Unknown { .. } => run_pdr(&m, &PdrConfig::default()).expect("pdr"),
        other => other,
    };
    assert!(
        matches!(res, ProveOutcome::Proved { .. }),
        "expected VIP handshake + APB DUT to be PROVED, got {res:?}"
    );
}
