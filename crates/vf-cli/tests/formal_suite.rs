use std::path::PathBuf;
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::equiv::check_equiv;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
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
fn bmc_finds_counterexample_and_writes_vcd() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_fail.sv")])).unwrap();
    let res = run_bmc(&m, &BmcConfig { depth: 10, solver: "z3".into() }).unwrap();
    match res {
        BmcOutcome::Fail { step, trace, .. } => {
            assert_eq!(step, 5, "counter should hit 5 at cycle 5");
            assert_eq!(trace.len(), 6);
            let wave = vcd::write_vcd(&m, &trace);
            assert!(wave.contains("$var wire 8"));
            assert!(wave.contains("count"));
        }
        other => panic!("expected Fail at cycle 5, got {other:?}"),
    }
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
fn cover_reaches_counter_max() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("counter_prove.sv")])).unwrap();
    let res = run_cover(&m, &CoverConfig { depth: 15, solver: "z3".into() }).unwrap();
    assert_eq!(res.len(), 1);
    let (step, trace) = res[0].reached.as_ref().expect("cover should be reached");
    assert_eq!(*step, 9);
    assert_eq!(trace.len(), 10);
}

#[test]
fn sync_fifo_proves_safety_and_covers_full() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("sync_fifo.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected sync_fifo to prove unboundedly, got {prove:?}"
    );
    let covers = run_cover(&m, &CoverConfig { depth: 10, solver: "z3".into() }).unwrap();
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
fn equiv_proves_ripple_adder_and_catches_bug() {
    let gold = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_gold.sv")])).unwrap();
    let gate = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_gate.sv")])).unwrap();
    let (_, ok) = check_equiv(&gold, &gate, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(
        matches!(ok, ProveOutcome::Proved { .. }),
        "expected ripple adder to be equivalent, got {ok:?}"
    );

    let bug = compile_to_ir(&CompileOptions::for_files(vec![fixture("adder_bug.sv")])).unwrap();
    let (_, bad) = check_equiv(&gold, &bug, &ProveConfig { max_k: 2, solver: "z3".into() }).unwrap();
    assert!(
        matches!(bad, ProveOutcome::Fail { step: 0, .. }),
        "expected buggy adder to fail at step 0, got {bad:?}"
    );
}

#[test]
fn memory_read_write_with_anyconst_proves() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("mem_rw.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected mem_rw to prove unboundedly, got {prove:?}"
    );
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
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected signed_fsm to prove unboundedly, got {prove:?}"
    );
    let covers = run_cover(&m, &CoverConfig { depth: 10, solver: "z3".into() }).unwrap();
    assert_eq!(covers.len(), 1);
    assert!(covers[0].reached.is_some(), "DONE state should be reachable");
}

#[test]
fn rr_arbiter_proves_mutex_work_conserving_and_fairness() {
    let m = compile_to_ir(&CompileOptions::for_files(vec![fixture("rr_arbiter.sv")])).unwrap();
    let prove = run_kinduction(&m, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(prove, ProveOutcome::Proved { .. }),
        "expected rr_arbiter to prove unboundedly, got {prove:?}"
    );
    let covers = run_cover(&m, &CoverConfig { depth: 5, solver: "z3".into() }).unwrap();
    assert_eq!(covers.len(), 2);
    assert!(covers.iter().all(|c| c.reached.is_some()));
}

#[test]
fn sequential_shift_register_equivalence_proves() {
    let gold = compile_to_ir(&CompileOptions::for_files(vec![fixture("shift_gold.sv")])).unwrap();
    let gate = compile_to_ir(&CompileOptions::for_files(vec![fixture("shift_gate.sv")])).unwrap();
    let (_, res) = check_equiv(&gold, &gate, &ProveConfig { max_k: 5, solver: "z3".into() }).unwrap();
    assert!(
        matches!(res, ProveOutcome::Proved { .. }),
        "expected shift_gold and shift_gate to be sequentially equivalent, got {res:?}"
    );
}

#[test]
fn cli_sby_runner_proves_sby_files() {
    let bin = env!("CARGO_BIN_EXE_veloformal");
    for sby in ["counter_prove.sby", "sync_fifo.sby"] {
        let out = std::process::Command::new(bin)
            .args(["sby", fixture(sby).to_str().unwrap()])
            .output()
            .expect("failed to execute veloformal binary");
        assert!(
            out.status.success(),
            "veloformal sby {sby} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("PROVED"), "expected PROVED in stdout: {stdout}");
    }
}

#[test]
fn differential_verdict_agreement_with_yosys() {
    // Only runs when yosys is installed on PATH.
    let has_yosys = std::process::Command::new("yosys")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_yosys {
        return;
    }

    for (file, top, mem_map) in [
        ("counter_prove.sv", "counter_prove", false),
        ("rr_arbiter.sv", "rr_arbiter", false),
        ("signed_fsm.sv", "signed_fsm", false),
        ("sync_fifo.sv", "sync_fifo", true),
    ] {
        let path = fixture(file);
        let m = compile_to_ir(&CompileOptions::for_files(vec![path.clone()])).unwrap();
        let vf = run_kinduction(&m, &ProveConfig { max_k: 10, solver: "z3".into() }).unwrap();
        assert!(matches!(vf, ProveOutcome::Proved { .. }), "VeloFormal failed on {file}");

        let mem_cmd = if mem_map { "memory_map; " } else { "" };
        let script = format!(
            "read_verilog -formal -sv {}; prep -top {top}; {mem_cmd}chformal -cover -remove; sat -tempinduct -prove-asserts",
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
