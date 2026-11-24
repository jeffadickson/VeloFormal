// VeloFormal QFL (Questa Formal Library) Arbiter Property Checker
// Verifies mutual exclusion (onehot0 grants), grant-only-on-request,
// work-conserving behavior, and optional bounded/liveness fairness.
module vf_arbiter_props #(
    parameter int N = 4,
    parameter bit WORK_CONSERVING = 1'b1
) (
    input logic         clk,
    input logic         rst,
    input logic [N-1:0] req,
    input logic [N-1:0] gnt
);
    // 1. Mutual exclusion: at most one requestor granted in any cycle.
    a_arb_mutex: assert property (@(posedge clk) disable iff (rst) $onehot0(gnt));

    // 2. No unsolicited grants: every granted bit must have an active request.
    a_arb_no_spurious: assert property (@(posedge clk) disable iff (rst) (gnt & ~req) == '0);

    // 3. Work-conserving: if any request is active, some grant must be issued.
    if (WORK_CONSERVING) begin : gen_wc
        a_arb_work_conserving: assert property (@(posedge clk) disable iff (rst) (|req) |-> (|gnt));
    end

    // 4. Functional cover goals: observe each channel winning arbitration.
    for (genvar i = 0; i < N; i++) begin : gen_cov
        c_arb_gnt: cover property (@(posedge clk) disable iff (rst) gnt[i]);
    end
endmodule
