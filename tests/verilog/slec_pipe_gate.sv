// SLEC implementation (gate): 2-stage pipelined datapath (latency = 2 cycles vs gold).
module slec_pipe_gate (
    input  logic       clk,
    input  logic [7:0] a,
    input  logic [7:0] b,
    input  logic [7:0] c,
    output logic [7:0] y
);
    logic [7:0] s1_sum;
    logic [7:0] s1_c;
    logic [7:0] s2_y;

    initial begin
        s1_sum = 8'd0;
        s1_c   = 8'd0;
        s2_y   = 8'd0;
    end

    always_ff @(posedge clk) begin
        s1_sum <= a + b;
        s1_c   <= c;
        s2_y   <= s1_sum ^ s1_c;
    end

    assign y = s2_y;
endmodule
