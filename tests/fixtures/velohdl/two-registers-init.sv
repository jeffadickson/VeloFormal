module two_registers_init (
    input logic clk,
    input logic rst_n,
    output logic [1:0] a,
    output logic [1:0] b
);
    initial begin a = 2'd0; b = 2'd0; end
    always_ff @(posedge clk) begin
        if (!rst_n) a <= 2'd0;
        else a <= a + 2'd1;
    end
    always_ff @(posedge clk) begin
        if (!rst_n) b <= 2'd0;
        else b <= a ^ 2'd3;
    end
`ifdef FORMAL
    always @* assume (rst_n);
    // b observes the prior value of a: at the first edge b becomes three.
    always_ff @(posedge clk) assert (b != 2'd3);
`endif
endmodule
