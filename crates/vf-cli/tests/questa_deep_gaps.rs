use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_veloformal"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ex21(name: &str) -> PathBuf {
    repo_root().join("examples/21_questa_parity").join(name)
}

fn run_cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(bin())
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
fn gap1_iso26262_fault_tmr_masked_parity_detected_and_spf() {
    // 1. TMR voter: every SA0/SA1/SEU fault in `r0` is 100% SAFE/MASKED (SPFM = 100.0%).
    let (ok, stdout, stderr) = run_cli(&[
        "fault",
        ex21("tmr_voter.sv").to_str().unwrap(),
        "--target",
        "r0",
        "--output",
        "safe_out",
    ]);
    assert!(ok, "TMR fault analysis should pass:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("SAFE/MASKED") && stdout.contains("SPFM=100.0%"),
        "expected 100% SAFE/MASKED TMR result:\n{stdout}"
    );
    assert!(
        stdout.contains("dangerous_spf=0"),
        "expected 0 dangerous SPFs on TMR:\n{stdout}"
    );

    // 2. Parity-protected register WITH `--alarm parity_err`: 100% DETECTED (DC=100.0%, SPFM=100.0%).
    let html_path = std::env::temp_dir().join("vf_fault_parity_report.html");
    let (ok2, stdout2, stderr2) = run_cli(&[
        "fault",
        ex21("parity_reg.sv").to_str().unwrap(),
        "--target",
        "data_reg",
        "--alarm",
        "parity_err",
        "--report",
        html_path.to_str().unwrap(),
    ]);
    assert!(ok2, "Parity fault analysis with alarm should pass:\n{stdout2}\n{stderr2}");
    assert!(
        stdout2.contains("DETECTED")
            && stdout2.contains("DC=100.0%")
            && stdout2.contains("SPFM=100.0%"),
        "expected 100% DETECTED parity coverage:\n{stdout2}"
    );
    let html = fs::read_to_string(&html_path).expect("read HTML FMEDA report");
    assert!(html.contains("ISO 26262 Formal Fault Analysis"));
    assert!(html.contains("DETECTED"));
    let _ = fs::remove_file(&html_path);

    // 3. Parity register WITHOUT `--alarm`: unprotected -> DANGEROUS Single-Point Faults (SPF)!
    let spf_html = std::env::temp_dir().join("vf_fault_spf_report.html");
    let (ok3, stdout3, _stderr3) = run_cli(&[
        "fault",
        ex21("parity_reg.sv").to_str().unwrap(),
        "--target",
        "data_reg",
        "--output",
        "dout",
        "--models",
        "sa0,seu",
        "--max-faults",
        "4",
        "--report",
        spf_html.to_str().unwrap(),
    ]);
    assert!(!ok3, "unprotected fault analysis should fail on SPF:\n{stdout3}");
    assert!(
        stdout3.contains("SPF") && stdout3.contains("SPFM=0.0%"),
        "expected SPF classification:\n{stdout3}"
    );
    let html3 = fs::read_to_string(&spf_html).expect("read SPF HTML");
    assert!(
        html3.contains("Root-Cause Signal Backtrace"),
        "expected Root-Cause Backtrace in SPF HTML report"
    );
    let _ = fs::remove_file(&spf_html);
}

#[test]
fn gap2_propcheck_deadend_detector_and_cex_backtrace() {
    // 1. Over-constrained FIFO hits dead-end at cycle 4.
    let (ok, stdout, stderr) = run_cli(&[
        "deadend",
        ex21("overconstrained_fifo.sv").to_str().unwrap(),
        "--depth",
        "8",
    ]);
    assert!(!ok, "expected dead-end failure:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("DEAD-END") && stdout.contains("cycle 4"),
        "expected dead-end at cycle 4:\n{stdout}"
    );
    assert!(
        stdout.contains("Root-Cause Backtrace"),
        "expected Root-Cause Backtrace in dead-end output:\n{stdout}"
    );

    // 2. Well-constrained design (always has valid input `push == 0` when full) has NO dead-end!
    let clean_sv = std::env::temp_dir().join("vf_clean_assume.sv");
    fs::write(
        &clean_sv,
        r#"
module clean_assume (input logic clk, input logic push, output logic [2:0] count);
    initial count = 3'd0;
    always_ff @(posedge clk) begin
        if (push && count < 3'd4) count <= count + 3'd1;
    end
    // Legal assumption: don't push when full (push=0 is always satisfiable!).
    assume property (@(posedge clk) !(count == 3'd4 && push));
    assert property (@(posedge clk) count <= 3'd4);
endmodule
"#,
    )
    .unwrap();
    let (ok2, stdout2, stderr2) = run_cli(&["deadend", clean_sv.to_str().unwrap(), "--depth", "8"]);
    let _ = fs::remove_file(&clean_sv);
    assert!(ok2, "clean assumption should have no dead-end:\n{stdout2}\n{stderr2}");
    assert!(stdout2.contains("CLEAN"), "expected CLEAN:\n{stdout2}");

    // 3. BMC counterexample prints Root-Cause Backtrace!
    let bug_sv = std::env::temp_dir().join("vf_bt_bug.sv");
    fs::write(
        &bug_sv,
        r#"
module bt_bug (input logic clk, input logic en, output logic [2:0] cnt);
    initial cnt = 3'd0;
    always_ff @(posedge clk) if (en) cnt <= cnt + 3'd1;
    a_small: assert property (@(posedge clk) cnt < 3'd3);
endmodule
"#,
    )
    .unwrap();
    let (ok3, stdout3, _) = run_cli(&["bmc", bug_sv.to_str().unwrap(), "--depth", "6"]);
    let _ = fs::remove_file(&bug_sv);
    assert!(!ok3);
    assert!(
        stdout3.contains("Root-Cause Backtrace") && stdout3.contains("cnt"),
        "expected Root-Cause Backtrace on BMC failure:\n{stdout3}"
    );
}

#[test]
fn gap3_autocheck_fsm_livelock_unreachable_shift_clock_const() {
    let (ok, stdout, stderr) = run_cli(&[
        "autocheck",
        ex21("fsm_livelock.sv").to_str().unwrap(),
        "--fsm",
        "state",
        "--check-const-regs",
        "--depth",
        "10",
    ]);
    assert!(!ok, "expected AutoCheck to find bugs:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("BUG  [fsm_livelock]"),
        "expected FSM livelock bug:\n{stdout}"
    );
    assert!(
        stdout.contains("WARN [fsm_unreachable]") && stdout.contains("state==4"),
        "expected FSM unreachable state 4 warning:\n{stdout}"
    );
    assert!(
        stdout.contains("BUG  [shift_overflow]"),
        "expected shift_overflow bug:\n{stdout}"
    );
    assert!(
        stdout.contains("WARN [clock_as_data]"),
        "expected clock_as_data warning:\n{stdout}"
    );
    assert!(
        stdout.contains("WARN [const_reg]") && stdout.contains("stuck_reg"),
        "expected const_reg warning for stuck_reg:\n{stdout}"
    );
}

#[test]
fn gap4_slec_reset_seq_lpgc_valid_cond_and_ignore_output() {
    let gold = ex21("lpgc_gold.sv");
    let gate = ex21("lpgc_gate.sv");

    // 1. WITH `--reset rst=1 --valid-cond out_valid --ignore-output cg_active`: PROVED EQUIVALENT!
    let (ok, stdout, stderr) = run_cli(&[
        "equiv",
        "--gold",
        gold.to_str().unwrap(),
        "--gate",
        gate.to_str().unwrap(),
        "--reset",
        "rst=1",
        "--reset-cycles",
        "1",
        "--valid-cond",
        "out_valid",
        "--ignore-output",
        "cg_active",
    ]);
    assert!(ok, "LPGC SLEC should prove equivalent:\n{stdout}\n{stderr}");
    assert!(stdout.contains("EQUIVALENT"), "expected EQUIVALENT:\n{stdout}");

    // 2. WITHOUT `--valid-cond out_valid`: clock-gated `out_data` holds stale value when idle -> NOT EQUIVALENT!
    let (ok2, stdout2, _stderr2) = run_cli(&[
        "equiv",
        "--gold",
        gold.to_str().unwrap(),
        "--gate",
        gate.to_str().unwrap(),
        "--reset",
        "rst=1",
        "--ignore-output",
        "cg_active",
    ]);
    assert!(!ok2, "without --valid-cond, LPGC designs should diverge on idle cycles:\n{stdout2}");
    assert!(
        stdout2.contains("NOT EQUIVALENT"),
        "expected NOT EQUIVALENT:\n{stdout2}"
    );
}

#[test]
fn gap5_upf_powercheck_clamps_pd_off_sequencing_and_bug_detection() {
    let upf = ex21("upf_iso.upf");

    // 1. Clean UPF DUT: clamp 0, clamp 1, clamp latch, AND `pd_off -> iso_en` sequencing all PASS!
    let (ok, stdout, stderr) = run_cli(&[
        "powercheck",
        ex21("upf_iso_dut.sv").to_str().unwrap(),
        "--upf",
        upf.to_str().unwrap(),
        "--pd-off",
        "PD_ACCEL=pd_off",
    ]);
    assert!(ok, "clean UPF powercheck should pass:\n{stdout}\n{stderr}");
    assert!(stdout.contains("clamp_0") && stdout.contains("clamp_1") && stdout.contains("clamp_latch"));
    assert!(stdout.contains("pd_off_iso_seq"));

    // 2. Buggy UPF DUT (missing clamp_0 on `pd_data`): FAIL caught!
    let (ok2, stdout2, _stderr2) = run_cli(&[
        "powercheck",
        ex21("upf_iso_buggy.sv").to_str().unwrap(),
        "--upf",
        upf.to_str().unwrap(),
    ]);
    assert!(!ok2, "buggy UPF DUT should fail powercheck:\n{stdout2}");
    assert!(
        stdout2.contains("FAIL") && stdout2.contains("pd_data"),
        "expected FAIL on pd_data:\n{stdout2}"
    );

    // 3. Structural un-isolated crossing check: UPF missing isolation rule for `pd_nready`!
    let uniso_upf = std::env::temp_dir().join("vf_uniso.upf");
    fs::write(
        &uniso_upf,
        r#"
create_power_domain PD_ACCEL -elements {pd_data pd_nready}
set_isolation iso_only_data -domain PD_ACCEL -clamp_value 0 -elements {pd_data} -isolation_signal iso_en -isolation_sense high
"#,
    )
    .unwrap();
    let (ok3, stdout3, _) = run_cli(&[
        "powercheck",
        ex21("upf_iso_dut.sv").to_str().unwrap(),
        "--upf",
        uniso_upf.to_str().unwrap(),
    ]);
    let _ = fs::remove_file(&uniso_upf);
    assert!(!ok3, "un-isolated domain crossing should fail:\n{stdout3}");
    assert!(
        stdout3.contains("VIOLATION") && stdout3.contains("pd_nready"),
        "expected VIOLATION for un-isolated pd_nready:\n{stdout3}"
    );
}

#[test]
fn gap6_regcheck_native_apb_and_registered_read_latency() {
    // 1. Native AMBA APB3 CSR block with SystemRDL 2.0 spec (`--protocol apb`).
    let (ok, stdout, stderr) = run_cli(&[
        "regcheck",
        ex21("apb_csr.sv").to_str().unwrap(),
        "--spec",
        ex21("apb_csr.rdl").to_str().unwrap(),
        "--protocol",
        "apb",
    ]);
    assert!(ok, "APB3 CSR regcheck should pass:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("PASS [CTRL] access_RW")
            && stdout.contains("PASS [STAT] access_RO")
            && stdout.contains("PASS [IRQ] access_W1C")
            && stdout.contains("PASS [KEY] access_WO"),
        "expected all APB CSR checks to pass:\n{stdout}"
    );

    // 2. Registered 1-cycle read latency CSR block (`--read-latency 1`).
    let reg_sv = std::env::temp_dir().join("vf_reg_lat1.sv");
    let reg_csv = std::env::temp_dir().join("vf_reg_lat1.csv");
    fs::write(
        &reg_sv,
        r#"
module csr_lat1 (
    input  logic       clk,
    input  logic       rst,
    input  logic       wr_en,
    input  logic       rd_en,
    input  logic [1:0] addr,
    input  logic [7:0] wdata,
    output logic [7:0] rdata
);
    logic [7:0] r0, r1;
    always_ff @(posedge clk) begin
        if (rst) begin
            r0 <= 8'h12;
            r1 <= 8'h34;
            rdata <= 8'h00;
        end else begin
            if (wr_en) begin
                if (addr == 2'd0) r0 <= wdata;
            end
            if (rd_en) begin
                rdata <= (addr == 2'd0) ? r0 : r1;
            end
        end
    end
endmodule
"#,
    )
    .unwrap();
    fs::write(
        &reg_csv,
        "name,addr,width,reset,access\nR0,0,8,0x12,RW\nR1,1,8,0x34,RO\n",
    )
    .unwrap();
    let (ok2, stdout2, stderr2) = run_cli(&[
        "regcheck",
        reg_sv.to_str().unwrap(),
        "--spec",
        reg_csv.to_str().unwrap(),
        "--read-latency",
        "1",
    ]);
    let _ = fs::remove_file(&reg_sv);
    let _ = fs::remove_file(&reg_csv);
    assert!(ok2, "1-cycle read-latency CSR regcheck should pass:\n{stdout2}\n{stderr2}");
    assert!(
        stdout2.contains("PASS [R0] reset")
            && stdout2.contains("PASS [R0] access_RW")
            && stdout2.contains("PASS [R1] access_RO"),
        "expected 1-cycle read-latency checks to pass:\n{stdout2}"
    );
}

#[test]
fn gap7_qfl_arbiter_and_gray_code_checkers_prove() {
    let qfl_sv = std::env::temp_dir().join("vf_qfl_dut.sv");
    fs::write(
        &qfl_sv,
        r#"
module qfl_dut (
    input  logic       clk,
    input  logic       rst,
    input  logic [3:0] req,
    input  logic       inc,
    output logic [3:0] gnt,
    output logic [3:0] gray
);
    // Fixed-priority work-conserving arbiter
    always_comb begin
        gnt = 4'b0000;
        if      (req[0]) gnt = 4'b0001;
        else if (req[1]) gnt = 4'b0010;
        else if (req[2]) gnt = 4'b0100;
        else if (req[3]) gnt = 4'b1000;
    end

    // 4-bit binary-to-Gray counter
    logic [3:0] bin;
    initial bin = 4'd0;
    always_ff @(posedge clk) begin
        if (rst) bin <= 4'd0;
        else if (inc) bin <= bin + 4'd1;
    end
    assign gray = bin ^ (bin >> 1);

    vf_arbiter_props #(.N(4)) u_arb_chk (
        .clk(clk), .rst(rst), .req(req), .gnt(gnt)
    );
    vf_gray_code #(.W(4)) u_gray_chk (
        .clk(clk), .rst(rst), .gray(gray)
    );
endmodule
"#,
    )
    .unwrap();

    let (ok, stdout, stderr) = run_cli(&[
        "prove",
        qfl_sv.to_str().unwrap(),
        repo_root().join("vip/vf_arbiter_props.sv").to_str().unwrap(),
        repo_root().join("vip/vf_gray_code.sv").to_str().unwrap(),
        "--top",
        "qfl_dut",
    ]);
    let _ = fs::remove_file(&qfl_sv);
    assert!(ok, "QFL arbiter + gray-code checkers should prove:\n{stdout}\n{stderr}");
    assert!(stdout.contains("PROVED"), "expected PROVED:\n{stdout}");
}

#[test]
fn questa_tcl_full_parity_script_passes_and_emits_html_dashboard() {
    let html_path = std::env::temp_dir().join("vf_questa_parity_tcl.html");
    let (ok, stdout, stderr) = run_cli(&[
        "tcl",
        ex21("run_questa_parity.do").to_str().unwrap(),
        "--report",
        html_path.to_str().unwrap(),
    ]);
    assert!(ok, "Questa parity Tcl script should pass:\n{stdout}\n{stderr}");
    let html = fs::read_to_string(&html_path).expect("read HTML dashboard");
    let _ = fs::remove_file(&html_path);
    assert!(html.contains("SAFE/MASKED"), "expected fault result in Tcl HTML");
    assert!(html.contains("powercheck"), "expected powercheck result in Tcl HTML");
    assert!(html.contains("regcheck"), "expected regcheck result in Tcl HTML");
    assert!(html.contains("slec_equiv"), "expected slec_equiv result in Tcl HTML");
}
