use std::fs;
use std::path::PathBuf;
use std::process::Command;
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_slang::CompileOptions;

fn write_temp_sv(name: &str, sv: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("vf_autocheck_expanded");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.sv"));
    fs::write(&path, sv).expect("write temp sv");
    path
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_veloformal")
}

#[test]
fn autocheck_formal_inferred_latch_catches_reachable_and_proves_unreachable_safe() {
    // 1. Reachable missing-else in always_comb -> BUG [inferred_latch] with CEX trace!
    let buggy_path = write_temp_sv(
        "latch_buggy",
        r#"
module latch_buggy (
    input  logic       clk,
    input  logic       en,
    input  logic [7:0] din,
    output logic [7:0] dout
);
    always_comb begin
        if (en) begin
            dout = din;
        end
    end
endmodule
"#,
    );
    let (_, findings) = run_autocheck(
        &CompileOptions::for_files(vec![buggy_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck latch_buggy");
    let latch_bug = findings.iter().find(|f| f.category == "inferred_latch");
    assert!(
        matches!(
            latch_bug,
            Some(f) if matches!(&f.status, CheckStatus::Bug { trace, .. } if !trace.is_empty())
        ),
        "expected formal inferred_latch BUG with CEX trace, got: {findings:#?}"
    );

    // 2. Missing default/else ONLY on formally unreachable state values -> SAFE [inferred_latch]!
    let unreachable_path = write_temp_sv(
        "latch_unreachable_safe",
        r#"
module latch_unreachable_safe (
    input  logic       clk,
    input  logic       rst,
    input  logic       toggle,
    output logic [7:0] y
);
    logic [1:0] st = 2'd0;
    always_ff @(posedge clk) begin
        if (rst) st <= 2'd0;
        else if (toggle) st <= (st == 2'd0) ? 2'd1 : 2'd0;
    end

    // States 2 and 3 are formally unreachable, so the missing else cannot infer a reachable latch!
    always_comb begin
        if (st == 2'd0) y = 8'hAA;
        else if (st == 2'd1) y = 8'h55;
    end
endmodule
"#,
    );
    let (_, safe_findings) = run_autocheck(
        &CompileOptions::for_files(vec![unreachable_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck latch_unreachable_safe");
    assert!(
        safe_findings
            .iter()
            .any(|f| f.category == "inferred_latch" && matches!(f.status, CheckStatus::Safe)),
        "expected unreachable missing-else to prove SAFE [inferred_latch], got: {safe_findings:#?}"
    );

    // 3. Intentional always_latch must NOT be flagged as an inferred_latch bug!
    let intentional_path = write_temp_sv(
        "latch_intentional",
        r#"
module latch_intentional (
    input  logic       en,
    input  logic [7:0] d,
    output logic [7:0] q
);
    always_latch begin
        if (en) q <= d;
    end
endmodule
"#,
    );
    let (_, int_findings) = run_autocheck(
        &CompileOptions::for_files(vec![intentional_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck latch_intentional");
    assert!(
        !int_findings.iter().any(|f| f.category == "inferred_latch"),
        "intentional always_latch should not emit inferred_latch finding: {int_findings:#?}"
    );
}

#[test]
fn autocheck_dynamic_bitselect_and_partselect_oob() {
    // 1. Unguarded dynamic bit-select read, write, and indexed part-select -> BUG [oob_bitselect]
    let buggy_path = write_temp_sv(
        "bitselect_oob_buggy",
        r#"
module bitselect_oob_buggy (
    input  logic       clk,
    input  logic [7:0] vec,
    input  logic [3:0] idx,
    input  logic       bit_in,
    output logic       bit_out,
    output logic [3:0] nib_out,
    output logic [7:0] vec_reg
);
    assign bit_out = vec[idx];
    assign nib_out = vec[idx +: 4];

    always_ff @(posedge clk) begin
        vec_reg[idx] <= bit_in;
    end
endmodule
"#,
    );
    let (_, findings) = run_autocheck(
        &CompileOptions::for_files(vec![buggy_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck bitselect_oob_buggy");
    let oob_bugs: Vec<_> = findings
        .iter()
        .filter(|f| f.category == "oob_bitselect" && matches!(f.status, CheckStatus::Bug { .. }))
        .collect();
    assert!(
        oob_bugs.len() >= 3,
        "expected >= 3 oob_bitselect bugs (read, part, write), got: {findings:#?}"
    );

    // 2. Properly guarded dynamic bit-select & part-select -> all SAFE!
    let clean_path = write_temp_sv(
        "bitselect_oob_clean",
        r#"
module bitselect_oob_clean (
    input  logic       clk,
    input  logic [7:0] vec,
    input  logic [3:0] idx,
    input  logic       bit_in,
    output logic       bit_out,
    output logic [3:0] nib_out,
    output logic [7:0] vec_reg
);
    assign bit_out = (idx < 4'd8) ? vec[idx] : 1'b0;
    assign nib_out = (idx <= 4'd4) ? vec[idx +: 4] : 4'd0;

    always_ff @(posedge clk) begin
        if (idx < 4'd8)
            vec_reg[idx] <= bit_in;
    end
endmodule
"#,
    );
    let (_, clean_findings) = run_autocheck(
        &CompileOptions::for_files(vec![clean_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck bitselect_oob_clean");
    assert!(
        clean_findings
            .iter()
            .filter(|f| f.category == "oob_bitselect")
            .all(|f| matches!(f.status, CheckStatus::Safe)),
        "expected all guarded oob_bitselect checks SAFE, got: {clean_findings:#?}"
    );
    assert!(
        clean_findings.iter().any(|f| f.category == "oob_bitselect"),
        "expected guarded oob_bitselect checks to have been emitted and proved SAFE"
    );
}

#[test]
fn autocheck_signed_and_multiply_overflow_plus_width_truncation() {
    // 1. Buggy signed add, signed sub, unsigned mul, and implicit narrowing truncation
    let buggy_path = write_temp_sv(
        "ovf_trunc_buggy",
        r#"
module ovf_trunc_buggy (
    input  logic signed [7:0]  sa,
    input  logic signed [7:0]  sb,
    input  logic        [7:0]  ua,
    input  logic        [7:0]  ub,
    input  logic        [15:0] wide_in,
    output logic signed [7:0]  s_sum,
    output logic signed [7:0]  s_diff,
    output logic        [7:0]  u_prod,
    output logic        [7:0]  narrow_out
);
    assign s_sum      = sa + sb;
    assign s_diff     = sa - sb;
    assign u_prod     = ua * ub;
    assign narrow_out = wide_in;
endmodule
"#,
    );
    let (_, findings) = run_autocheck(
        &CompileOptions::for_files(vec![buggy_path]),
        &AutoCheckConfig {
            check_overflow: true,
            check_truncation: true,
            ..Default::default()
        },
    )
    .expect("autocheck ovf_trunc_buggy");
    let ovf_bugs = findings
        .iter()
        .filter(|f| f.category == "overflow" && matches!(f.status, CheckStatus::Bug { .. }))
        .count();
    assert!(
        ovf_bugs >= 3,
        "expected >= 3 overflow bugs (sadd, ssub, mul), got: {findings:#?}"
    );
    assert!(
        findings.iter().any(|f| {
            f.category == "width_truncation" && matches!(f.status, CheckStatus::Bug { .. })
        }),
        "expected width_truncation bug, got: {findings:#?}"
    );

    // 2. Clean guarded signed/mul + explicit slice & masked narrowing -> all SAFE!
    let clean_path = write_temp_sv(
        "ovf_trunc_clean",
        r#"
module ovf_trunc_clean (
    input  logic signed [7:0]  sa,
    input  logic signed [7:0]  sb,
    input  logic        [7:0]  ua,
    input  logic        [7:0]  ub,
    input  logic        [15:0] wide_in,
    output logic signed [7:0]  s_sum,
    output logic        [7:0]  u_prod,
    output logic        [7:0]  masked_narrow,
    output logic        [7:0]  explicit_slice
);
    always_comb begin
        if (sa >= -8'sd32 && sa <= 8'sd31 && sb >= -8'sd32 && sb <= 8'sd31)
            s_sum = sa + sb;
        else
            s_sum = 8'sd0;

        if (ua <= 8'd15 && ub <= 8'd15)
            u_prod = ua * ub;
        else
            u_prod = 8'd0;
    end
    assign masked_narrow  = wide_in & 16'h00FF;
    assign explicit_slice = wide_in[7:0];
endmodule
"#,
    );
    let (_, clean_findings) = run_autocheck(
        &CompileOptions::for_files(vec![clean_path]),
        &AutoCheckConfig {
            check_overflow: true,
            check_truncation: true,
            ..Default::default()
        },
    )
    .expect("autocheck ovf_trunc_clean");
    for f in &clean_findings {
        assert!(
            matches!(f.status, CheckStatus::Safe),
            "expected all checks SAFE on guarded arithmetic/truncation, got: {f:#?}"
        );
    }
}

#[test]
fn autocheck_comb_loop_and_multi_driver_without_crashing() {
    // 1. Module containing a combinational loop (a -> b -> a), multi-driven comb+seq signal,
    //    AND a div-by-zero bug: AutoCheck must report ALL three without crashing!
    let buggy_path = write_temp_sv(
        "comb_loop_and_multidrv",
        r#"
module comb_loop_and_multidrv (
    input  logic       clk,
    input  logic [7:0] x,
    input  logic [7:0] denom,
    output logic [7:0] a,
    output logic [7:0] b,
    output logic [7:0] dual_sig,
    output logic [7:0] quot
);
    assign a = b ^ x;
    assign b = a & x;

    assign dual_sig = x;
    always_ff @(posedge clk) begin
        dual_sig <= x + 8'd1;
    end

    assign quot = x / denom;
endmodule
"#,
    );
    let (_, findings) = run_autocheck(
        &CompileOptions::for_files(vec![buggy_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck should NOT crash on comb loop");
    assert!(
        findings
            .iter()
            .any(|f| f.category == "comb_loop" && matches!(f.status, CheckStatus::Bug { .. })),
        "expected comb_loop BUG finding, got: {findings:#?}"
    );
    assert!(
        findings.iter().any(|f| {
            f.category == "multi_driven"
                && f.name == "dual_sig"
                && matches!(f.status, CheckStatus::Bug { .. })
        }),
        "expected multi_driven BUG for dual_sig, got: {findings:#?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "div_by_zero" && matches!(f.status, CheckStatus::Bug { .. })),
        "expected div_by_zero BUG to still be checked after breaking comb loop: {findings:#?}"
    );

    // 2. Disjoint bit-slice continuous assigns must NOT trigger multi_driven!
    let disjoint_path = write_temp_sv(
        "disjoint_slices_clean",
        r#"
module disjoint_slices_clean (
    input  logic [3:0] lo,
    input  logic [3:0] hi,
    output logic [7:0] bus
);
    assign bus[3:0] = lo;
    assign bus[7:4] = hi;
endmodule
"#,
    );
    let (_, disj_findings) = run_autocheck(
        &CompileOptions::for_files(vec![disjoint_path]),
        &AutoCheckConfig::default(),
    )
    .expect("autocheck disjoint_slices_clean");
    assert!(
        !disj_findings.iter().any(|f| f.category == "multi_driven"),
        "disjoint slice assigns must not trigger multi_driven: {disj_findings:#?}"
    );
}

#[test]
fn autocheck_auto_fsm_no_reset_reg_and_unread_signal() {
    let sv_path = write_temp_sv(
        "auto_fsm_reset_unread",
        r#"
module auto_fsm_reset_unread (
    input  logic       clk,
    input  logic       rst,
    input  logic       go,
    input  logic [7:0] din,
    output logic [7:0] dout,
    output logic [1:0] mode_out
);
    // FSM state register discovered automatically by --auto-fsm (state 2 deadlocks, state 3 unreachable)
    logic [1:0] ctrl_state = 2'd0;
    // Properly reset uninitialized register
    logic [7:0] good_reg;
    // Uninitialized register FORGOTTEN in reset branch -> no_reset_reg!
    logic [7:0] forgot_reset_reg;
    // Driven internal wire never read anywhere -> unread_signal!
    logic [7:0] dead_wire;

    assign dead_wire = din ^ 8'hFF;

    always_ff @(posedge clk) begin
        if (rst) begin
            ctrl_state <= 2'd0;
            good_reg   <= 8'd0;
        end else begin
            good_reg         <= din;
            forgot_reset_reg <= din + 8'd1;
            case (ctrl_state)
                2'd0: if (go) ctrl_state <= 2'd1;
                2'd1: ctrl_state <= 2'd2;
                2'd2: ctrl_state <= 2'd2; // deadlock!
                2'd3: ctrl_state <= 2'd0; // unreachable!
            endcase
        end
    end

    assign dout     = good_reg ^ forgot_reset_reg;
    assign mode_out = ctrl_state;
endmodule
"#,
    );

    let (_, findings) = run_autocheck(
        &CompileOptions::for_files(vec![sv_path]),
        &AutoCheckConfig {
            depth: 10,
            auto_fsm: true,
            check_no_reset: true,
            check_unread: true,
            ..Default::default()
        },
    )
    .expect("autocheck auto_fsm_reset_unread");

    assert!(
        findings.iter().any(|f| {
            f.category == "fsm_deadlock"
                && f.name == "ctrl_state"
                && matches!(f.status, CheckStatus::Bug { .. })
        }),
        "expected auto-discovered FSM deadlock on ctrl_state, got: {findings:#?}"
    );
    assert!(
        findings.iter().any(|f| {
            f.category == "fsm_unreachable"
                && f.name == "ctrl_state==3"
                && matches!(f.status, CheckStatus::Warning(_))
        }),
        "expected auto-discovered unreachable state 3, got: {findings:#?}"
    );
    assert!(
        findings.iter().any(|f| {
            f.category == "no_reset_reg"
                && f.name == "forgot_reset_reg"
                && matches!(f.status, CheckStatus::Warning(_))
        }),
        "expected no_reset_reg warning on forgot_reset_reg, got: {findings:#?}"
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.category == "no_reset_reg" && f.name == "good_reg"),
        "good_reg IS reset and must not trigger no_reset_reg: {findings:#?}"
    );
    assert!(
        findings.iter().any(|f| {
            f.category == "unread_signal"
                && f.name == "dead_wire"
                && matches!(f.status, CheckStatus::Warning(_))
        }),
        "expected unread_signal warning on dead_wire, got: {findings:#?}"
    );
}

#[test]
fn autocheck_cli_strict_enable_disable_json_sarif_html_and_questa_tcl() {
    let dir = std::env::temp_dir().join("vf_autocheck_cli_test");
    let _ = fs::create_dir_all(&dir);
    let sv_path = dir.join("cli_target.sv");
    fs::write(
        &sv_path,
        r#"
module cli_target (
    input  logic       clk,
    input  logic       en,
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] q_latch,
    output logic [7:0] q_div
);
    always_comb begin
        if (en) q_latch = a;
    end
    assign q_div = a / b;
endmodule
"#,
    )
    .unwrap();

    let html_path = dir.join("autocheck.html");
    let sarif_path = dir.join("autocheck.sarif");
    let junit_path = dir.join("autocheck.xml");

    // 1. JSON + HTML + SARIF + JUnit output with --strict
    let out = Command::new(cli_bin())
        .args([
            "autocheck",
            sv_path.to_str().unwrap(),
            "--strict",
            "--format",
            "json",
            "--report",
            html_path.to_str().unwrap(),
            "--sarif",
            sarif_path.to_str().unwrap(),
            "--junit",
            junit_path.to_str().unwrap(),
        ])
        .output()
        .expect("run veloformal autocheck");
    assert!(!out.status.success(), "expected non-zero exit on bugs");
    let json_val: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("valid JSON output");
    assert_eq!(json_val["module"], "cli_target");
    assert!(json_val["summary"]["bugs"].as_u64().unwrap_or(0) >= 2);
    assert!(fs::read_to_string(&html_path).unwrap().contains("AutoCheck"));
    assert!(fs::read_to_string(&sarif_path).unwrap().contains("2.1.0"));
    assert!(fs::read_to_string(&junit_path).unwrap().contains("testsuite"));

    // 2. --disable inferred_latch --disable div_by_zero -> exit code 0!
    let out_waived = Command::new(cli_bin())
        .args([
            "autocheck",
            sv_path.to_str().unwrap(),
            "--disable",
            "inferred_latch",
            "--disable",
            "div_by_zero",
        ])
        .output()
        .expect("run veloformal autocheck with waivers");
    assert!(
        out_waived.status.success(),
        "expected waived autocheck to pass, stderr={}",
        String::from_utf8_lossy(&out_waived.stderr)
    );

    // 3. Questa Tcl `autocheck disable -type ...` + `autocheck verify`
    let tcl_path = dir.join("run_ac.tcl");
    fs::write(
        &tcl_path,
        format!(
            "formal compile -d cli_target {}\nautocheck disable -type inferred_latch\nautocheck disable -type div_by_zero\nautocheck verify -depth 6\n",
            sv_path.display()
        ),
    )
    .unwrap();
    let out_tcl = Command::new(cli_bin())
        .args(["tcl", tcl_path.to_str().unwrap()])
        .output()
        .expect("run veloformal tcl");
    assert!(
        out_tcl.status.success(),
        "expected Questa Tcl autocheck with disable waivers to pass:\n{}\n{}",
        String::from_utf8_lossy(&out_tcl.stdout),
        String::from_utf8_lossy(&out_tcl.stderr)
    );
}
