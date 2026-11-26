use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use vf_engine::equiv::{check_slec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig, PdrStrategy};
use vf_engine::portfolio::{
    run_portfolio, run_portfolio_decomposed, run_portfolio_monolithic, PortfolioConfig,
};
use vf_ir::opt::sweep_sequential;
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::smt2;
use vf_smt::solver::{CancelToken, SmtSolver};

fn compile_sv_inline(name: &str, sv: &str) -> Module {
    let dir = std::env::temp_dir().join("vf_scaling_suite");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.sv"));
    fs::write(&path, sv).expect("write temp sv");
    let m = compile_to_ir(&CompileOptions::for_files(vec![path.clone()]))
        .unwrap_or_else(|e| panic!("failed to compile {name}.sv: {e:#}"));
    let _ = fs::remove_file(&path);
    m
}

fn repo_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
        .canonicalize()
        .expect("repo file")
}

#[test]
fn smt2_dag_sharing_prevents_exponential_blowup() {
    // 20-stage deep 32-bit combinational diamond DAG inside a single `always_comb` block:
    // Each procedural stage references the previous `ExprId` twice (`2^20 = 1,048,576` naive tree
    // leaves in ONE expression!). Without DAG sharing, `encode_step` would emit > 50 MB of SMT-LIB2
    // and OOM/timeout. With O(N) SMT-LIB2 Tseitin (`declare-fun` + `assert =`) DAG sharing,
    // `encode_step` stays linear (< 32 KB) and incremental Z3 proves `out_a == out_b` in milliseconds.
    let mut sv = String::from(
        r#"
module deep_diamond_dag (
    input  logic        clk,
    input  logic [31:0] a,
    input  logic [31:0] b,
    output logic [31:0] out_a,
    output logic [31:0] out_b
);
    logic [31:0] s;
    always_comb begin
        s = a ^ b;
"#,
    );
    for _ in 1..=20 {
        sv.push_str("        s = (s + 32'd1) ^ {1'b0, s[31:1]};\n");
    }
    sv.push_str(
        r#"        out_a = s + 32'd5;
        out_b = 32'd5 + s;
        p_dag_equiv: assert (out_a == out_b);
    end
endmodule
"#,
    );

    let m = compile_sv_inline("deep_diamond_dag", &sv);
    let step_smt = smt2::encode_step(&m, 0, true, None);
    assert!(
        step_smt.contains("(declare-fun e_"),
        "expected O(N) SMT-LIB2 Tseitin DAG sharing in encode_step"
    );
    assert!(
        step_smt.len() < 32_000,
        "SMT-LIB2 step blew up! got {} bytes (expected < 32 KB with DAG sharing)",
        step_smt.len()
    );

    let t0 = Instant::now();
    let res = run_kinduction(
        &m,
        &ProveConfig {
            max_k: 4,
            solver: "z3".into(),
        },
    )
    .expect("k-induction failed");
    assert!(
        matches!(res, ProveOutcome::Proved { .. }),
        "expected PROVED, got {res:?}"
    );
    assert!(
        t0.elapsed().as_secs() < 5,
        "deep diamond DAG took too long: {:?}",
        t0.elapsed()
    );
}

#[test]
fn ir_sequential_constant_and_duplicate_ff_sweeping() {
    // 1. Design with 4 constant-stuck initialized config FFs + 3 cascaded duplicate 32-bit
    //    pipeline FF pairs + 1 live counter FF.
    let sv = r#"
module sweep_target (
    input  logic        clk,
    input  logic        rst,
    input  logic        en,
    input  logic [31:0] din,
    output logic [31:0] pipe_a2,
    output logic [31:0] pipe_b2,
    output logic [7:0]  cfg_out
);
    logic [7:0]  cfg_mode = 8'h5A;
    logic [7:0]  cfg_mask = 8'hFF;
    logic [7:0]  cfg_bias = 8'h00;
    logic        cfg_en   = 1'b1;

    logic [31:0] p0_a = 32'd0;
    logic [31:0] p0_b = 32'd0;
    logic [31:0] p1_a = 32'd0;
    logic [31:0] p1_b = 32'd0;
    logic [31:0] p2_a = 32'd0;
    logic [31:0] p2_b = 32'd0;
    logic [7:0]  cnt  = 8'd0;

    always_ff @(posedge clk) begin
        if (rst) begin
            cfg_mode <= 8'h5A;
            cfg_mask <= 8'hFF;
            cfg_bias <= 8'h00;
            cfg_en   <= 1'b1;
            p0_a     <= 32'd0;
            p0_b     <= 32'd0;
            p1_a     <= 32'd0;
            p1_b     <= 32'd0;
            p2_a     <= 32'd0;
            p2_b     <= 32'd0;
            cnt      <= 8'd0;
        end else begin
            // Constant-preserving hold registers
            cfg_mode <= cfg_mode;
            cfg_mask <= en ? 8'hFF : cfg_mask;
            cfg_bias <= cfg_bias;
            cfg_en   <= 1'b1;

            // Cascaded duplicate 32-bit pipelines
            p0_a <= (din + {24'd0, cfg_bias}) & {24'hFFFFFF, cfg_mask};
            p0_b <= (din + {24'd0, cfg_bias}) & {24'hFFFFFF, cfg_mask};
            p1_a <= p0_a ^ 32'hA5A5_5A5A;
            p1_b <= p0_b ^ 32'hA5A5_5A5A;
            p2_a <= p1_a + 32'd1;
            p2_b <= p1_b + 32'd1;

            if (en && cfg_en && cnt < 8'd10)
                cnt <= cnt + 8'd1;
        end
    end

    assign pipe_a2 = p2_a;
    assign pipe_b2 = p2_b;
    assign cfg_out = cfg_mode;

    always @(posedge clk) begin
        p_cfg:  assert (cfg_out == 8'h5A);
        p_pipe: assert (pipe_a2 == pipe_b2);
        p_cnt:  assert (cnt <= 8'd10);
    end
endmodule
"#;

    let mut m = compile_sv_inline("sweep_target", sv);
    let ffs_before = m.ffs.len();
    let stats = sweep_sequential(&mut m);
    assert!(
        stats.const_ffs_removed >= 4,
        "expected >= 4 constant FFs removed, got {stats:?}"
    );
    assert!(
        stats.dup_ffs_merged >= 3,
        "expected >= 3 cascaded duplicate FFs merged, got {stats:?}"
    );
    assert!(
        m.ffs.len() <= ffs_before - 7,
        "expected FFs reduced by >= 7 (before={ffs_before}, after={})",
        m.ffs.len()
    );

    let res = run_kinduction(
        &m,
        &ProveConfig {
            max_k: 4,
            solver: "z3".into(),
        },
    )
    .expect("k-induction failed");
    assert!(
        matches!(res, ProveOutcome::Proved { .. }),
        "expected PROVED after sequential sweeping, got {res:?}"
    );

    // 2. Soundness check: UNINITIALIZED duplicate FFs MUST NOT be merged!
    let uninit_sv = r#"
module uninit_no_merge (
    input logic       clk,
    input logic [7:0] din
);
    logic [7:0] u1;
    logic [7:0] u2;

    always_ff @(posedge clk) begin
        u1 <= din;
        u2 <= din;
    end

    always @(posedge clk) begin
        p_should_fail: assert (u1 == u2);
    end
endmodule
"#;
    let mut m_uninit = compile_sv_inline("uninit_no_merge", uninit_sv);
    let u_stats = sweep_sequential(&mut m_uninit);
    assert_eq!(
        u_stats.dup_ffs_merged, 0,
        "uninitialized FFs must NEVER be merged (soundness violation!)"
    );
    let u_res = run_portfolio(&m_uninit, &PortfolioConfig::default()).expect("portfolio failed");
    assert!(
        matches!(u_res.outcome, ProveOutcome::Fail { step: 0, .. }),
        "uninitialized FFs must fail at cycle 0, got {:?}",
        u_res.outcome
    );
}

#[test]
fn abc_simplified_pdr_interpolation_and_portfolio_cancellation() {
    let pdr_sv = repo_file("tests/verilog/pdr_non_inductive.sv");
    let m = compile_to_ir(&CompileOptions::for_files(vec![pdr_sv])).expect("compile pdr test");

    // 1. ABC AIG pre-simplification (`strash; dc2; scorr -C 500; dc2; pdr`)
    let r_pdr = run_pdr(
        &m,
        &PdrConfig {
            strategy: PdrStrategy::PdrSimplified,
            ..Default::default()
        },
    )
    .expect("simplified PDR failed");
    assert!(
        matches!(r_pdr, ProveOutcome::Proved { .. }),
        "expected simplified PDR PROVED, got {r_pdr:?}"
    );

    // 2. ABC Craig interpolation (`strash; dc2; int -k`)
    let r_int = run_pdr(
        &m,
        &PdrConfig {
            strategy: PdrStrategy::Interpolation,
            ..Default::default()
        },
    )
    .expect("ABC interpolation failed");
    assert!(
        matches!(r_int, ProveOutcome::Proved { .. }),
        "expected ABC interpolation PROVED, got {r_int:?}"
    );

    // 3. Monolithic 3-engine portfolio races kind + pdr(abc-dc2) + int(abc-dc2) with CancelToken
    let r_port = run_portfolio_monolithic(&m, &PortfolioConfig::default()).expect("portfolio");
    assert!(
        matches!(r_port.outcome, ProveOutcome::Proved { .. }),
        "expected portfolio PROVED, got {r_port:?}"
    );

    // 4. Direct CancelToken verification: killing a live Z3 child unblocks immediately
    let tok = CancelToken::new();
    let mut solver = SmtSolver::spawn_with_cancel("z3", Some(tok.clone())).expect("spawn z3");
    solver
        .send("(declare-const x (_ BitVec 32))\n(assert (= x #x00000001))\n")
        .expect("send");
    tok.cancel();
    assert!(tok.is_cancelled());
    // Subsequent check_sat must fail cleanly because Z3 was terminated & reaped by CancelToken.
    assert!(
        solver.check_sat().is_err(),
        "expected cancelled Z3 solver to report closed pipe"
    );
}

fn generate_multichannel_soc_sv(inject_ch3_bug: bool) -> String {
    let mut sv = String::from(
        r#"
module multichannel_dma_alu_soc (
    input  logic        clk,
    input  logic        rst,
    input  logic [3:0]  req_valid,
    input  logic [3:0]  ack_ready,
    input  logic [31:0] din0, din1, din2, din3,
    input  logic [1:0]  alu_op,
    output logic [31:0] alu_out,
    output logic [3:0]  ch_busy
);
    // Shared 32-bit multi-stage ALU datapath (128 bits of pipeline state)
    logic [31:0] alu_s0 = 32'd0;
    logic [31:0] alu_s1 = 32'd0;
    logic [31:0] alu_s2 = 32'd0;
    logic [31:0] alu_mirror = 32'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            alu_s0     <= 32'd0;
            alu_s1     <= 32'd0;
            alu_s2     <= 32'd0;
            alu_mirror <= 32'd0;
        end else begin
            case (alu_op)
                2'd0: alu_s0 <= din0 + din1;
                2'd1: alu_s0 <= din0 ^ din1;
                2'd2: alu_s0 <= din2 + din3;
                default: alu_s0 <= din2 ^ din3;
            endcase
            alu_s1     <= (alu_s0 + 32'd7) ^ {1'b0, alu_s0[31:1]};
            alu_s2     <= alu_s1 + 32'd1;
            alu_mirror <= alu_s1 + 32'd1;
        end
    end
    assign alu_out = alu_s2;

"#,
    );

    // 4 independent 32-bit DMA/credit channels (~140 FF bits per channel = ~560 FF bits)
    for ch in 0..4 {
        let max_cred_check = if ch == 3 && inject_ch3_bug {
            "8'd16" // Bug: allows credit to reach 16, violating `<= 8'd15`!
        } else {
            "8'd15"
        };
        sv.push_str(&format!(
            r#"
    logic [31:0] ch{ch}_p0 = 32'd0;
    logic [31:0] ch{ch}_p1 = 32'd0;
    logic [31:0] ch{ch}_p2 = 32'd0;
    logic [31:0] ch{ch}_p3 = 32'd0;
    logic [7:0]  ch{ch}_cred = 8'd0;
    logic        ch{ch}_active = 1'b0;
    logic        ch{ch}_full   = 1'b0;

    always_ff @(posedge clk) begin
        if (rst) begin
            ch{ch}_p0     <= 32'd0;
            ch{ch}_p1     <= 32'd0;
            ch{ch}_p2     <= 32'd0;
            ch{ch}_p3     <= 32'd0;
            ch{ch}_cred   <= 8'd0;
            ch{ch}_active <= 1'b0;
            ch{ch}_full   <= 1'b0;
        end else begin
            ch{ch}_p0 <= din{ch} ^ 32'h0000_000{ch};
            ch{ch}_p1 <= ch{ch}_p0 + 32'd1;
            ch{ch}_p2 <= ch{ch}_p1 ^ 32'h55AA_00FF;
            ch{ch}_p3 <= ch{ch}_p2 + 32'd3;

            if (req_valid[{ch}] && !ack_ready[{ch}] && ch{ch}_cred < {max_cred_check}) begin
                ch{ch}_cred   <= ch{ch}_cred + 8'd1;
                ch{ch}_active <= 1'b1;
                ch{ch}_full   <= (ch{ch}_cred + 8'd1 == 8'd15);
            end else if (!req_valid[{ch}] && ack_ready[{ch}] && ch{ch}_cred > 8'd0) begin
                ch{ch}_cred   <= ch{ch}_cred - 8'd1;
                ch{ch}_active <= (ch{ch}_cred - 8'd1 > 8'd0);
                ch{ch}_full   <= 1'b0;
            end
        end
    end
    assign ch_busy[{ch}] = ch{ch}_active;
"#
        ));
    }

    // 12 independent assertions across the 4 channels + ALU
    sv.push_str(
        r#"
    always @(posedge clk) begin
        p_alu_mirror: assert (alu_s2 == alu_mirror);
        p_ch0_bound:  assert (ch0_cred <= 8'd15);
        p_ch0_act:    assert (ch0_active == (ch0_cred != 8'd0));
        p_ch0_full:   assert (ch0_full == (ch0_cred == 8'd15));
        p_ch1_bound:  assert (ch1_cred <= 8'd15);
        p_ch1_act:    assert (ch1_active == (ch1_cred != 8'd0));
        p_ch1_full:   assert (ch1_full == (ch1_cred == 8'd15));
        p_ch2_bound:  assert (ch2_cred <= 8'd15);
        p_ch2_act:    assert (ch2_active == (ch2_cred != 8'd0));
        p_ch3_bound:  assert (ch3_cred <= 8'd15);
        p_ch3_act:    assert (ch3_active == (ch3_cred != 8'd0));
        p_ch3_full:   assert (ch3_full == (ch3_cred == 8'd15));
    end
endmodule
"#,
    );
    sv
}

#[test]
fn per_property_parallel_coi_large_multichannel_soc_and_lemma_promotion() {
    // Part A: Large ~700-FF-bit 4-channel DMA + 32-bit ALU SoC with 12 assertions!
    let clean_sv = generate_multichannel_soc_sv(false);
    let m_clean = compile_sv_inline("multichannel_soc_clean", &clean_sv);
    let total_ff_bits: u32 = m_clean.ffs.iter().map(|f| m_clean.signal(f.signal).width).sum();
    assert!(
        total_ff_bits >= 600,
        "expected large SoC fixture with >= 600 FF bits, got {total_ff_bits}"
    );

    let t0 = Instant::now();
    let res = run_portfolio(&m_clean, &PortfolioConfig::default()).expect("decomposed portfolio");
    assert!(
        matches!(res.outcome, ProveOutcome::Proved { .. }),
        "expected large multi-channel SoC PROVED, got {res:?}"
    );
    assert!(
        res.winner.contains("decomposed(12 props)"),
        "expected decomposed(12 props) winner, got {}",
        res.winner
    );
    assert!(
        t0.elapsed().as_secs() < 15,
        "12-assertion large SoC took too long: {:?}",
        t0.elapsed()
    );

    // Part B: Fail-fast detection on Channel 3 bug!
    let buggy_sv = generate_multichannel_soc_sv(true);
    let m_buggy = compile_sv_inline("multichannel_soc_buggy", &buggy_sv);
    let res_bug =
        run_portfolio(&m_buggy, &PortfolioConfig::default()).expect("buggy decomposed portfolio");
    match res_bug.outcome {
        ProveOutcome::Fail {
            property, trace, ..
        } => {
            assert!(
                property.contains("p_ch3_"),
                "expected Channel 3 assertion to fail, got '{property}'"
            );
            assert!(!trace.is_empty(), "expected non-empty word-level CEX trace");
        }
        other => panic!("expected Fail on buggy Channel 3, got {other:?}"),
    }

    // Part C: Sound lemma promotion fallback (with k-induction only, depth=2):
    // `p_tight` (`cnt <= 5`) is 1-inductive by itself;
    // `p_dependent` (`flag == 1'b0`, where `flag` sets only if `cnt == 8`) is NOT 1-inductive
    // individually, so phase 1 proves `p_tight`, promotes `p_tight` to `Assume`, and phase 2
    // proves `p_dependent` immediately!
    let lemma_sv = r#"
module lemma_promotion_dut (
    input logic clk,
    input logic rst,
    input logic inc
);
    logic [7:0] cnt  = 8'd0;
    logic       flag = 1'b0;

    always_ff @(posedge clk) begin
        if (rst) begin
            cnt  <= 8'd0;
            flag <= 1'b0;
        end else begin
            if (inc && cnt < 8'd5)
                cnt <= cnt + 8'd1;
            if (cnt == 8'd8)
                flag <= 1'b1;
        end
    end

    always @(posedge clk) begin
        p_tight:     assert (cnt <= 8'd5);
        p_dependent: assert (!flag);
    end
endmodule
"#;
    let m_lemma = compile_sv_inline("lemma_promotion_dut", lemma_sv);
    let r_lemma = run_portfolio_decomposed(
        &m_lemma,
        &PortfolioConfig {
            depth: 2,
            solver: "z3".into(),
            enable_pdr: false,
            cache_dir: None,
        },
    )
    .expect("lemma promotion failed");
    assert!(
        matches!(r_lemma.outcome, ProveOutcome::Proved { .. }),
        "expected lemma promotion PROVED, got {r_lemma:?}"
    );
    assert!(
        r_lemma.winner.contains("decomposed+lemma("),
        "expected decomposed+lemma fallback, got {}",
        r_lemma.winner
    );
}

#[test]
fn slec_large_refactored_pipeline_ff_merging() {
    let gold_sv = r#"
module pipe_quad_gold (
    input  logic        clk,
    input  logic        rst,
    input  logic [31:0] a, b, c, d,
    output logic [31:0] y0, y1, y2, y3
);
    logic [31:0] r0 = 32'd0, r1 = 32'd0, r2 = 32'd0, r3 = 32'd0;
    logic [31:0] s0 = 32'd0, s1 = 32'd0, s2 = 32'd0, s3 = 32'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            r0 <= 0; r1 <= 0; r2 <= 0; r3 <= 0;
            s0 <= 0; s1 <= 0; s2 <= 0; s3 <= 0;
        end else begin
            r0 <= a + b;
            r1 <= b ^ c;
            r2 <= c + d;
            r3 <= (a + b) ^ (c + d);
            s0 <= r0 + 32'd1;
            s1 <= r1 + 32'd2;
            s2 <= r2 + 32'd3;
            s3 <= r3 + r0;
        end
    end
    assign y0 = s0; assign y1 = s1; assign y2 = s2; assign y3 = s3;
endmodule
"#;
    let gate_sv = r#"
module pipe_quad_gate (
    input  logic        clk,
    input  logic        rst,
    input  logic [31:0] a, b, c, d,
    output logic [31:0] y0, y1, y2, y3
);
    logic [31:0] r0 = 32'd0, r1 = 32'd0, r2 = 32'd0, r3 = 32'd0;
    logic [31:0] s0 = 32'd0, s1 = 32'd0, s2 = 32'd0, s3 = 32'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            r0 <= 0; r1 <= 0; r2 <= 0; r3 <= 0;
            s0 <= 0; s1 <= 0; s2 <= 0; s3 <= 0;
        end else begin
            r0 <= b + a;
            r1 <= c ^ b;
            r2 <= d + c;
            r3 <= (d + c) ^ (b + a);
            s0 <= 32'd1 + r0;
            s1 <= 32'd2 + r1;
            s2 <= 32'd3 + r2;
            s3 <= r0 + r3;
        end
    end
    assign y0 = s0; assign y1 = s1; assign y2 = s2; assign y3 = s3;
endmodule
"#;

    let gold = compile_sv_inline("pipe_quad_gold", gold_sv);
    let gate = compile_sv_inline("pipe_quad_gate", gate_sv);
    let (_, outcome) = check_slec(
        &gold,
        &gate,
        &SlecConfig {
            engine: "kind".into(),
            depth: 6,
            ..Default::default()
        },
    )
    .expect("SLEC failed");
    assert!(
        matches!(outcome, ProveOutcome::Proved { .. }),
        "expected SLEC PROVED on refactored 4-channel 32-bit pipeline, got {outcome:?}"
    );
}
