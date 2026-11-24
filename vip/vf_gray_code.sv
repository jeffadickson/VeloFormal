// VeloFormal QFL (Questa Formal Library) Gray-Code Transition Checker
// Proves that at most 1 bit changes between consecutive clock cycles
// (critical for CDC FIFO pointers and glitch-free state encodings).
module vf_gray_code #(
    parameter int W = 4
) (
    input logic         clk,
    input logic         rst,
    input logic [W-1:0] gray
);
    logic seen_valid;
    initial seen_valid = 1'b0;
    always_ff @(posedge clk) begin
        if (rst) seen_valid <= 1'b0;
        else     seen_valid <= 1'b1;
    end

    logic [W-1:0] diff;
    assign diff = gray ^ $past(gray);

    // Single-bit transition invariant: Hamming distance <= 1 between consecutive cycles.
    a_gray_single_bit: assert property (
        @(posedge clk) disable iff (rst) seen_valid |-> ($countones(diff) <= 1)
    );

    // Non-vacuity cover: observe an actual 1-bit transition.
    c_gray_step: cover property (
        @(posedge clk) disable iff (rst) seen_valid && ($countones(diff) == 1)
    );
endmodule
