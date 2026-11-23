// Example 09 (Gate - Buggy): Pipeline delay bug where `c` is only delayed by 1 cycle instead of 2!
// Caught immediately by `veloformal equiv --gold mac_gold.sv --gate mac_pipe3_bug.sv --latency-gold 3`.
module mac_pipe3_bug (
    input  logic       clk,
    input  logic [7:0] a,
    input  logic [7:0] b,
    input  logic [7:0] c,
    output logic [7:0] y
);
    logic [7:0] s1_a, s1_b, s1_c;
    logic [7:0] s2_prod;
    logic [7:0] s3_y;

    initial begin
        s1_a    = 8'd0;
        s1_b    = 8'd0;
        s1_c    = 8'd0;
        s2_prod = 8'd0;
        s3_y    = 8'd0;
    end

    always_ff @(posedge clk) begin
        s1_a    <= a;
        s1_b    <= b;
        s1_c    <= c;
        s2_prod <= s1_a * s1_b;
        // BUG: uses s1_c (1 cycle behind) instead of s2_c (2 cycles behind)!
        s3_y    <= s2_prod + s1_c;
    end

    assign y = s3_y;
endmodule
