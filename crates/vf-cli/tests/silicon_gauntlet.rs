use std::path::PathBuf;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_veloformal")
}

fn ex23(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/23_silicon_gauntlet")
        .join(rel)
        .canonicalize()
        .expect("example 23 file")
}

fn run_vf(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(cli_bin())
        .args(args)
        .output()
        .expect("run veloformal");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn silicon_gauntlet_ot_prim_fifo_pkg_2d_packed_array_and_compound_assign() {
    let sv = ex23("ot_prim_fifo_sync.sv");
    let vfc = ex23("ot_prim_fifo_sync.vfc");

    // 1. AutoCheck clean on OpenTitan-style FIFO (2D packed array, package, enum, struct, +=, inside)
    let (ac_ok, ac_out, ac_err) = run_vf(&[
        "autocheck",
        sv.to_str().unwrap(),
        "--top",
        "ot_prim_fifo_sync",
    ]);
    assert!(ac_ok, "AutoCheck failed on ot_prim_fifo_sync:\n{ac_out}\n{ac_err}");

    // 2. Contract proof + vacuity
    let (ok, out, err) = run_vf(&[
        "contract",
        sv.to_str().unwrap(),
        "--top",
        "ot_prim_fifo_sync",
        "--spec",
        vfc.to_str().unwrap(),
        "--vacuity",
    ]);
    assert!(ok, "Contract failed on ot_prim_fifo_sync:\n{out}\n{err}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");
    assert!(out.contains("NON-VACUOUS"), "{out}");
}

#[test]
fn silicon_gauntlet_ot_arbiter_ppc_instance_array_streaming_and_case_inside() {
    let sv = ex23("ot_prim_arbiter_ppc.sv");
    let vfc = ex23("ot_prim_arbiter_ppc.vfc");

    let (ac_ok, ac_out, ac_err) = run_vf(&[
        "autocheck",
        sv.to_str().unwrap(),
        "--top",
        "ot_prim_arbiter_ppc",
    ]);
    assert!(ac_ok, "AutoCheck failed on ot_prim_arbiter_ppc:\n{ac_out}\n{ac_err}");

    let (ok, out, err) = run_vf(&[
        "contract",
        sv.to_str().unwrap(),
        "--top",
        "ot_prim_arbiter_ppc",
        "--spec",
        vfc.to_str().unwrap(),
        "--vacuity",
    ]);
    assert!(ok, "Contract failed on ot_prim_arbiter_ppc:\n{out}\n{err}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");
    assert!(out.contains("NON-VACUOUS"), "{out}");
}

#[test]
fn silicon_gauntlet_tlul_interface_and_modport_crossbar() {
    let sv = ex23("tlul_if_crossbar.sv");
    let vfc = ex23("tlul_if_crossbar.vfc");

    let (ac_ok, ac_out, ac_err) = run_vf(&[
        "autocheck",
        sv.to_str().unwrap(),
        "--top",
        "tlul_if_crossbar",
    ]);
    assert!(ac_ok, "AutoCheck failed on tlul_if_crossbar:\n{ac_out}\n{ac_err}");

    let (ok, out, err) = run_vf(&[
        "contract",
        sv.to_str().unwrap(),
        "--top",
        "tlul_if_crossbar",
        "--spec",
        vfc.to_str().unwrap(),
        "--vacuity",
    ]);
    assert!(ok, "Contract failed on tlul_if_crossbar:\n{out}\n{err}");
    assert!(out.contains("PROVED"), "{out}");
    assert!(out.contains("REACHED"), "{out}");
    assert!(out.contains("NON-VACUOUS"), "{out}");
}
