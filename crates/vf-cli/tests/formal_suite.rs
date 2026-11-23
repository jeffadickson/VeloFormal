use std::path::PathBuf;
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::equiv::check_equiv;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::live::{run_live, LiveConfig, LiveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::passes::{async2sync, chformal, clk2fflogic, ChformalAction};
use vf_ir::PropertyKind;
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::{btor2, smt2, vcd};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("verilog")
        .join(name)
}

#[test]
fn bmc_finds_counterexample_with_skip_append_and_vcd() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_fail.sv")])).unwrap();
    let res = run_bmc(
        &m,
        &BmcConfig {
            depth: 10,
            solver: "z3".into(),
            skip: 0,
            append: 2,
        },
    )
    .unwrap();
    match res {
        BmcOutcome::Fail { step, trace, .. } => {
            assert_eq!(step, 5, "counter should hit 5 at cycle 5");
            assert_eq!(trace.len(), 8, "6 steps (0..=5) + 2 appended cycles");
            let wave = vcd::write_vcd(&m, &trace);
            assert!(wave.contains("$var wire 8"));
            assert!(wave.contains("count"));
        }
        other => panic!("expected Fail at cycle 5, got {other:?}"),
    }

    // With skip=6, cycle 5 is skipped so failure is first detected at cycle 6.
    let skipped = run_bmc(
        &m,
        &BmcConfig {
            depth: 10,
            solver: "z3".into(),
            skip: 6,
            append: 0,
        },
    )
    .unwrap();
    assert!(matches!(skipped, BmcOutcome::Fail { step: 6, .. }));
}

#[test]
fn kinduction_proves_modulo_counter_unbounded() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_prove.sv")])).unwrap();
    let res = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
    match res {
        ProveOutcome::Proved { k } => assert!(k <= 2, "expected small k, got {k}"),
        other => panic!("expected Proved, got {other:?}"),
    }
}

#[test]
fn pdr_proves_non_inductive_invariant_that_kinduction_cannot() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("pdr_non_inductive.sv")])).unwrap();
    // k-induction cannot prove this for small k because `a != b` is unreachable but not asserted!
    let k_res = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(k_res, ProveOutcome::Unknown { .. }),
        "expected k-induction to be Unknown on non-inductive invariant, got {k_res:?}"
    );
    // ABC PDR discovers `a == b` automatically and proves `a != 15` unboundedly!
    let pdr_res = run_pdr(&m, &PdrConfig::default()).unwrap();
    assert!(
        matches!(pdr_res, ProveOutcome::Proved { .. }),
        "expected ABC PDR to prove non-inductive invariant, got {pdr_res:?}"
    );
}

#[test]
fn pdr_catches_bug_with_word_level_counterexample() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_fail.sv")])).unwrap();
    let res = run_pdr(&m, &PdrConfig::default()).unwrap();
    match res {
        ProveOutcome::Fail { step, trace, .. } => {
            assert_eq!(step, 5);
            assert_eq!(trace.len(), 6);
        }
        other => panic!("expected PDR Fail at cycle 5, got {other:?}"),
    }
}

#[test]
fn liveness_and_fairness_finds_fair_starving_lasso() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("liveness_starve.sv")])).unwrap();
    let res = run_live(&m, &LiveConfig { depth: 8, solver: "z3".into() }).unwrap();
    match res {
        LiveOutcome::Fail {
            loop_start,
            loop_end,
            trace,
            ..
        } => {
            assert!(loop_start < loop_end);
            assert_eq!(trace.len(), loop_end + 1);
        }
        other => panic!("expected fair starving lasso Fail, got {other:?}"),
    }
}

#[test]
fn cover_reaches_counter_max_with_append() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_prove.sv")])).unwrap();
    let res = run_cover(
        &m,
        &CoverConfig {
            depth: 15,
            solver: "z3".into(),
            append: 2,
        },
    )
    .unwrap();
    assert_eq!(res.len(), 1);
    let (step, trace) = res[0].reached.as_ref().expect("cover should be reached");
    assert_eq!(*step, 9);
    assert_eq!(trace.len(), 12, "10 steps (0..=9) + 2 appended cycles");
}

#[test]
fn sync_fifo_proves_safety_and_covers_full() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("sync_fifo.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected sync_fifo to prove unboundedly, got {prove:?}"
    );
    let covers = run_cover(&m, &CoverConfig::default()).unwrap();
    assert_eq!(covers.len(), 1);
    let (step, _) = covers[0].reached.as_ref().expect("FIFO full should be reachable");
    assert_eq!(*step, 4);
}

#[test]
fn hierarchy_case_and_sva_implication_proves() {
    let mut opts = CompileOptions::for_files(vec![fixture("hierarchy_alu.sv")]);
    opts.top = Some("alu_top".to_string());
    let m = compile_to_ir(&opts).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected alu_top SVA property to prove, got {prove:?}"
    );
}

#[test]
fn sva_multicycle_delay_proves() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("sva_delay.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected sva_delay `req |-> ##2 ack` to prove, got {prove:?}"
    );
}

#[test]
fn casez_wildcard_decoder_proves() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("casez_decoder.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
}

#[test]
fn procedural_for_loop_popcount_proves() {
    let m =
        compile_to_ir(&CompileOptions::for_files(vec![fixture("for_loop_popcount.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
}

#[test]
fn packed_struct_sv_function_and_anyconst_attr_proves() {
    let m =
        compile_to_ir(&CompileOptions::for_files(vec![fixture("struct_and_func.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
}

#[test]
fn async_reset_with_async2sync_proves() {
    let mut m =
        compile_to_ir(&CompileOptions::for_files(vec![fixture("async_reset_reg.sv")])).unwrap();
    async2sync(&mut m);
    let prove = run_kinduction(&m, &ProveConfig { max_k: 3, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected async_reset_reg with async2sync to prove, got {prove:?}"
    );
}

#[test]
fn multiclock_clk2fflogic_proves() {
    let mut m =
        compile_to_ir(&CompileOptions::for_files(vec![fixture("multiclock_toggle.sv")])).unwrap();
    clk2fflogic(&mut m);
    let prove = run_kinduction(&m, &ProveConfig { max_k: 4, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected multiclock_toggle with clk2fflogic to prove, got {prove:?}"
    );
}

#[test]
fn chformal_pass_transforms_properties() {
    let mut m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_fail.sv")])).unwrap();
    chformal(&mut m, Some(PropertyKind::Assert), ChformalAction::Remove);
    assert!(m.properties.iter().all(|p| p.kind != PropertyKind::Assert));
    let res = run_bmc(&m, &BmcConfig::default()).unwrap();
    assert!(matches!(res, BmcOutcome::Pass { .. }));
}

#[test]
fn equiv_proves_ripple_adder_and_catches_bug() {
    let gold = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_gold.sv")])).unwrap();
    let gate = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_gate.sv")])).unwrap();
    let (_, ok) = check_equiv(&gold, &gate, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(matches!(ok, ProveOutcome::Proved { .. }));

    let bug = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_bug.sv")])).unwrap();
    let (_, bad) = check_equiv(&gold, &bug, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(matches!(bad, ProveOutcome::Fail { step: 0, .. }));
}

#[test]
fn memory_read_write_with_anyconst_proves() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("mem_rw.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
    let btor = btor2::export_btor2(&m);
    assert!(btor.contains("sort array"));
    assert!(btor.contains("write"));
    let smt = smt2::dump_standalone_smt2(&m);
    assert!(smt.contains("QF_ABV"));
}

#[test]
fn signed_fsm_proves_onehot_and_signed_bounds() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("signed_fsm.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
    let covers = run_cover(&m, &CoverConfig::default()).unwrap();
    assert_eq!(covers.len(), 1);
    assert!(covers[0].reached.is_some());
}

#[test]
fn rr_arbiter_proves_mutex_work_conserving_and_fairness() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("rr_arbiter.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(matches!(prove, ProveOutcome::Proved { .. }));
    let covers = run_cover(&m, &CoverConfig::default()).unwrap();
    assert_eq!(covers.len(), 2);
    assert!(covers.iter().all(|c| c.reached.is_some()));
}

#[test]
fn sequential_shift_register_equivalence_proves() {
    let gold = compile_to_ir(&CompileOptions::for_files(vec![fixture("shift_gold.sv")])).unwrap();
    let gate = compile_to_ir(&CompileOptions::for_files(vec![fixture("shift_gate.sv")])).unwrap();
    let (_, res) = check_equiv(&gold, &gate, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(matches!(res, ProveOutcome::Proved { .. }));
}

#[test]
fn cli_sby_multi_task_with_inline_files_pdr_cover_and_expect_fail() {
    let bin = env!("CARGO_BIN_EXE_veloformal");
    let workdir = std::env::temp_dir().join(format!("vf_sby_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);

    let out = std::process::Command::new(bin)
        .args([
            "sby",
            fixture("multi_task.sby").to_str().unwrap(),
            "-d",
            workdir.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute veloformal binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "veloformal sby multi_task.sby failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("SBY [bmc] PASS"));
    assert!(stdout.contains("SBY [prove] PASS"));
    assert!(stdout.contains("SBY [pdr] PASS"));
    assert!(stdout.contains("SBY [cover] PASS"));
    assert!(stdout.contains("SBY [expect_fail] FAIL"));
    assert!(workdir.join("pdr/status").exists());
    assert!(workdir.join("cover/engine_0/trace0.vcd").exists());
    assert!(workdir.join("expect_fail/engine_0/trace.vcd").exists());

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn differential_verdict_agreement_with_yosys() {
    let has_yosys = std::process::Command::new("yosys")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_yosys {
        return;
    }

    // Yosys >= 0.39 emits `$check` cells and requires `chformal -lower` before `sat`.
    let needs_lower = std::process::Command::new("yosys")
        .args(["-p", "help chformal"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("-lower"))
        .unwrap_or(false);
    let lower_cmd = if needs_lower { "chformal -lower; " } else { "" };

    for (file, top, mem_map) in [
        ("counter_prove.sv", "counter_prove", false),
        ("rr_arbiter.sv", "rr_arbiter", false),
        ("signed_fsm.sv", "signed_fsm", false),
        ("sync_fifo.sv", "sync_fifo", true),
        ("casez_decoder.sv", "casez_decoder", false),
        ("for_loop_popcount.sv", "for_loop_popcount", false),
    ] {
        let path = fixture(file);
        let m = compile_to_ir(&CompileOptions::for_files(vec![path.clone()])).unwrap();
        let vf = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
        assert!(matches!(vf, ProveOutcome::Proved { .. }), "VeloFormal failed on {file}");

        let mem_cmd = if mem_map { "memory_map; " } else { "" };
        let script = format!(
            "read_verilog -formal -sv {}; prep -top {top}; {mem_cmd}{lower_cmd}chformal -cover -remove; sat -tempinduct -prove-asserts",
            path.display()
        );
        let yo = std::process::Command::new("yosys")
            .args(["-q", "-p", &script])
            .output()
            .expect("failed to run yosys");
        assert!(
            yo.status.success(),
            "Yosys disagreed on {file}:\n{}",
            String::from_utf8_lossy(&yo.stderr)
        );
    }
}
