use std::path::PathBuf;
use std::process::Command;

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
fn example_01_traffic_light_sby() {
    let (code, out) = run_vf(&["sby", "examples/01_traffic_light_fsm/traffic_light.sby"]);
    assert_eq!(code, 0, "example 01 failed:\n{out}");
    assert!(out.contains("SBY [prove] PASS"), "{out}");
    assert!(out.contains("SBY [bmc] PASS"), "{out}");
    assert!(out.contains("SBY [cover] PASS"), "{out}");
}

#[test]
fn example_02_skid_buffer_vip() {
    let (code, out) = run_vf(&[
        "prove",
        "vip/vf_handshake.sv",
        "examples/02_skid_buffer_vip/skid_buffer.sv",
        "--top",
        "skid_buffer",
    ]);
    assert_eq!(code, 0, "example 02 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");

    let (code, out) = run_vf(&[
        "cover",
        "vip/vf_handshake.sv",
        "examples/02_skid_buffer_vip/skid_buffer.sv",
        "--top",
        "skid_buffer",
        "--depth",
        "10",
    ]);
    assert_eq!(code, 0, "example 02 cover failed:\n{out}");
    assert!(out.contains("COVER REACHED"), "{out}");
}

#[test]
fn example_03_fifo_scoreboard_vip() {
    let (code, out) = run_vf(&[
        "prove",
        "vip/vf_fifo_props.sv",
        "examples/03_fifo_scoreboard_vip/sync_fifo_vip.sv",
        "--top",
        "sync_fifo_vip",
        "--engine",
        "auto",
        "--depth",
        "8",
    ]);
    assert_eq!(code, 0, "example 03 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
}

#[test]
fn example_04_axi4_lite_slave_vip() {
    let (code, out) = run_vf(&[
        "prove",
        "vip/vf_handshake.sv",
        "vip/vf_axi4_lite.sv",
        "examples/04_axi4_lite_slave_vip/axi4_lite_slave.sv",
        "--top",
        "axi4_lite_slave",
    ]);
    assert_eq!(code, 0, "example 04 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
}

#[test]
fn example_05_apb_bridge_vip() {
    let (code, out) = run_vf(&[
        "prove",
        "vip/vf_apb.sv",
        "examples/05_apb_bridge_vip/apb_bridge_vip.sv",
        "--top",
        "apb_bridge_vip",
    ]);
    assert_eq!(code, 0, "example 05 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");
}

#[test]
fn example_06_arbiter_liveness() {
    let (code, out) = run_vf(&["prove", "examples/06_arbiter_liveness/rr_arbiter_4.sv"]);
    assert_eq!(code, 0, "example 06 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");

    let (code, out) = run_vf(&[
        "live",
        "examples/06_arbiter_liveness/rr_arbiter_4.sv",
        "--depth",
        "8",
    ]);
    assert_eq!(code, 0, "example 06 rr live failed:\n{out}");
    assert!(out.contains("PASS"), "{out}");

    let (code, out) = run_vf(&[
        "live",
        "examples/06_arbiter_liveness/fixed_prio_starve.sv",
        "--depth",
        "6",
    ]);
    assert_eq!(code, 1, "example 06 fixed prio expected FAIL:\n{out}");
    assert!(out.contains("FAIL"), "{out}");
}

#[test]
fn example_07_uart_tx_sva() {
    let (code, out) = run_vf(&[
        "prove",
        "examples/07_uart_tx_sva/uart_tx.sv",
        "--engine",
        "auto",
        "--depth",
        "12",
    ]);
    assert_eq!(code, 0, "example 07 prove failed:\n{out}");
    assert!(out.contains("PROVED"), "{out}");

    let (code, out) = run_vf(&[
        "cover",
        "examples/07_uart_tx_sva/uart_tx.sv",
        "--depth",
        "15",
    ]);
    assert_eq!(code, 0, "example 07 cover failed:\n{out}");
    assert!(out.contains("COVER REACHED"), "{out}");
}

#[test]
fn example_08_pdr_credit_counter() {
    let (code, out) = run_vf(&["sby", "examples/08_pdr_credit_counter/credit_flow.sby"]);
    assert_eq!(code, 0, "example 08 sby failed:\n{out}");
    assert!(out.contains("SBY [prove_pdr] PASS"), "{out}");
    assert!(out.contains("SBY [cover] PASS"), "{out}");
}

#[test]
fn example_09_slec_pipelined_mac() {
    let (code, out) = run_vf(&[
        "equiv",
        "--gold",
        "examples/09_slec_pipelined_mac/mac_gold.sv",
        "--gate",
        "examples/09_slec_pipelined_mac/mac_pipe3.sv",
        "--latency-gold",
        "3",
    ]);
    assert_eq!(code, 0, "example 09 equiv failed:\n{out}");
    assert!(out.contains("EQUIVALENT"), "{out}");

    let (code, out) = run_vf(&[
        "equiv",
        "--gold",
        "examples/09_slec_pipelined_mac/mac_gold.sv",
        "--gate",
        "examples/09_slec_pipelined_mac/mac_pipe3_bug.sv",
        "--latency-gold",
        "3",
    ]);
    assert_eq!(code, 1, "example 09 buggy expected NOT EQUIVALENT:\n{out}");
    assert!(out.contains("NOT EQUIVALENT"), "{out}");
}

#[test]
fn example_10_autocheck_packet_parser() {
    let (code, out) = run_vf(&[
        "autocheck",
        "examples/10_autocheck_packet_parser/packet_parser_buggy.sv",
        "--fsm",
        "parse_state",
    ]);
    assert_eq!(code, 1, "example 10 buggy expected bugs:\n{out}");
    assert!(out.contains("BUG  [oob_memory]"), "{out}");
    assert!(out.contains("BUG  [div_by_zero]"), "{out}");
    assert!(out.contains("BUG  [unique_priority]"), "{out}");
    assert!(out.contains("BUG  [fsm_deadlock]"), "{out}");

    let (code, out) = run_vf(&[
        "autocheck",
        "examples/10_autocheck_packet_parser/packet_parser_fixed.sv",
        "--fsm",
        "parse_state",
    ]);
    assert_eq!(code, 0, "example 10 fixed expected SAFE:\n{out}");
    assert!(!out.contains("BUG"), "{out}");
}

#[test]
fn example_11_covercheck_alu_ctrl() {
    let (code, out) = run_vf(&[
        "covercheck",
        "examples/11_covercheck_alu_ctrl/alu_controller.sv",
    ]);
    assert_eq!(code, 0, "example 11 covercheck failed:\n{out}");
    assert!(out.contains("REACHABLE"), "{out}");
    assert!(out.contains("UNREACHABLE"), "{out}");
}

#[test]
fn example_12_connectivity_soc_pinmux() {
    let (code, out) = run_vf(&[
        "connectivity",
        "examples/12_connectivity_soc_pinmux/soc_pinmux.sv",
        "--spec",
        "examples/12_connectivity_soc_pinmux/pinmux_rules.csv",
    ]);
    assert_eq!(code, 0, "example 12 connectivity failed:\n{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

#[test]
fn example_13_secure_key_vault() {
    let (code, out) = run_vf(&[
        "secure",
        "examples/13_secure_key_vault/key_vault.sv",
        "--source",
        "root_key",
        "--target",
        "telemetry_out",
    ]);
    assert_eq!(code, 0, "example 13 telemetry expected SECURE:\n{out}");
    assert!(out.contains("SECURE"), "{out}");

    let (code, out) = run_vf(&[
        "secure",
        "examples/13_secure_key_vault/key_vault.sv",
        "--source",
        "root_key",
        "--target",
        "jtag_rdata",
    ]);
    assert_eq!(code, 1, "example 13 jtag without declassify expected LEAK:\n{out}");
    assert!(out.contains("LEAK"), "{out}");

    let (code, out) = run_vf(&[
        "secure",
        "examples/13_secure_key_vault/key_vault.sv",
        "--source",
        "root_key",
        "--target",
        "jtag_rdata",
        "--declassify",
        "jtag_unlocked && auth_ok",
    ]);
    assert_eq!(code, 0, "example 13 jtag with declassify expected SECURE:\n{out}");
    assert!(out.contains("SECURE"), "{out}");
}

#[test]
fn example_14_xcheck_dma_reset() {
    let (code, out) = run_vf(&[
        "xcheck",
        "examples/14_xcheck_dma_reset/dma_channel.sv",
        "--target",
        "valid_out",
        "--target",
        "masked_data",
    ]);
    assert_eq!(code, 0, "example 14 valid/masked expected X-FREE:\n{out}");
    assert!(out.contains("X-FREE `valid_out`"), "{out}");
    assert!(out.contains("X-FREE `masked_data`"), "{out}");

    let (code, out) = run_vf(&[
        "xcheck",
        "examples/14_xcheck_dma_reset/dma_channel.sv",
        "--target",
        "status_out",
    ]);
    assert_eq!(code, 1, "example 14 status_out expected X-LEAK:\n{out}");
    assert!(out.contains("X-LEAK `status_out`"), "{out}");
}

#[test]
fn example_15_regcheck_uart_csr() {
    let (code, out) = run_vf(&[
        "regcheck",
        "examples/15_regcheck_uart_csr/uart_csr.sv",
        "--spec",
        "examples/15_regcheck_uart_csr/uart_csr.csv",
    ]);
    assert_eq!(code, 0, "example 15 regcheck failed:\n{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

#[test]
fn example_16_cdc_async_bridge() {
    let (code, out) = run_vf(&[
        "cdc",
        "examples/16_cdc_async_bridge/async_bridge.sv",
        "--gray",
        "wr_ptr_gray",
    ]);
    assert_eq!(code, 0, "example 16 clean cdc failed:\n{out}");
    assert!(out.contains("SAFE  [synchronized_cdc]"), "{out}");
    assert!(out.contains("GRAY PROVED `wr_ptr_gray`"), "{out}");

    let (code, out) = run_vf(&["cdc", "examples/16_cdc_async_bridge/async_bridge_buggy.sv"]);
    assert_eq!(code, 1, "example 16 buggy cdc expected errors:\n{out}");
    assert!(out.contains("ERROR [multi_bit_cdc]"), "{out}");
    assert!(out.contains("ERROR [comb_logic_cdc]"), "{out}");
    assert!(out.contains("WARN  [cdc_reconvergence]"), "{out}");
    assert!(out.contains("WARN  [rdc_crossing]"), "{out}");
}

#[test]
fn example_17_vacuity_detection() {
    let (code, out) = run_vf(&[
        "vacuity",
        "examples/17_vacuity_detection/arbiter_vacuous.sv",
    ]);
    assert_eq!(code, 1, "example 17 expected vacuous assertion:\n{out}");
    assert!(out.contains("NON-VACUOUS"), "{out}");
    assert!(out.contains("VACUOUS"), "{out}");
}
