// Example 09 (Gate - Equivalent): 3-stage pipelined MAC datapath.
// Proved bit-for-bit equivalent to mac_gold using `veloformal equiv --latency-gold 3`.
module mac_pipe3 (
    input  logic       clk,
    input  logic [7:0] a,
    input  logic [7:0] b,
    input  logic [7:0] c,
    output logic [7:0] y
);
    // Stage 1: register operands
    logic [7:0] s1_a, s1_b, s1_c;
    // Stage 2: partial product + delayed c
    logic [7:0] s2_prod, s2_c;
    // Stage 3: final sum
    logic [7:0] s3_y;

    initial begin
        s1_a    = 8'd0;
        s1_b    = 8'd0;
        s1_c    = 8'd0;
        s2_prod = 8'd0;
        s2_c    = 8'd0;
        s3_y    = 8'd0;
    end

    always_ff @(posedge clk) begin
        s1_a    <= a;
        s1_b    <= b;
        s1_c    <= c;
        s2_prod <= s1_a * s1_b;
        s2_c    <= s1_c;
        s3_y    <= s2_prod + s2_c;
    end

    assign y = s3_y;
endmodule
