// Example 09 (Gold): Behavioral single-cycle datapath `(a * b) + c` (8-bit truncated).
module mac_gold (
    input  logic       clk,
    input  logic [7:0] a,
    input  logic [7:0] b,
    input  logic [7:0] c,
    output logic [7:0] y
);
    assign y = (a * b) + c;
endmodule
