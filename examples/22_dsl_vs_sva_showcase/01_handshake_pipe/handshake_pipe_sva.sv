// Tier 1 — Traditional IEEE 1800 SVA `bind` Spec (~56 lines of boilerplate)
// Pain points:
//  1. Must manually re-declare every RTL port in a wrapper module + `bind` statement.
//  2. Must manually track `f_past_valid` so `$past(...)` doesn't spuriously fail at cycle 0.
//  3. Must repeat `@(posedge clk) disable iff (rst)` on every single assertion and cover.
//  4. Must hand-code ready/valid backpressure stability with cryptic `|=>` + `$past` + `$stable`.
module handshake_pipe_sva (
    input logic       clk,
    input logic       rst,
    input logic       in_valid,
    input logic       in_ready,
    input logic [7:0] in_data,
    input logic       out_valid,
    input logic       out_ready,
    input logic [7:0] out_data
);
    // Boilerplate #1: Manual cycle-0 guard so $past() doesn't fail on the first clock edge!
    logic f_past_valid;
    initial f_past_valid = 1'b0;
    always_ff @(posedge clk) begin
        if (rst) f_past_valid <= 1'b0;
        else     f_past_valid <= 1'b1;
    end

    // Boilerplate #2: Hand-coded ready/valid backpressure stability (easy to get wrong!)
    a_valid_stable_on_stall: assert property (
        @(posedge clk) disable iff (rst)
        (f_past_valid && $past(out_valid) && !$past(out_ready)) |-> out_valid
    );

    a_data_stable_on_stall: assert property (
        @(posedge clk) disable iff (rst)
        (f_past_valid && $past(out_valid) && !$past(out_ready)) |-> $stable(out_data)
    );

    // Boilerplate #3: Pipeline functional transformation check
    a_accept_transforms_data: assert property (
        @(posedge clk) disable iff (rst)
        (in_valid && in_ready) |=> (out_valid && out_data == ($past(in_data) + 8'd1))
    );

    // Boilerplate #4: Backpressure readiness invariant
    a_ready_when_empty: assert property (
        @(posedge clk) disable iff (rst)
        !out_valid |-> in_ready
    );

    // Boilerplate #5: Reachability covers
    c_handshake_transfer: cover property (
        @(posedge clk) disable iff (rst)
        out_valid && out_ready && (out_data == 8'h42)
    );
endmodule

bind handshake_pipe handshake_pipe_sva u_handshake_pipe_sva (
    .clk(clk), .rst(rst),
    .in_valid(in_valid), .in_ready(in_ready), .in_data(in_data),
    .out_valid(out_valid), .out_ready(out_ready), .out_data(out_data)
);
