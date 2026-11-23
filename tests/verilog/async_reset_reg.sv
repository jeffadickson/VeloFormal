module async_reset_reg (
    input  logic       clk,
    input  logic       rst_n,
    input  logic [7:0] d,
    output logic [7:0] q
);
    initial q = 8'h00;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            q <= 8'h00;
        else
            q <= d;
    end

`ifdef FORMAL
    // With async2sync, asserting !rst_n forces q == 0 immediately in the same cycle!
    always_comb begin
        if (!rst_n)
            assert (q == 8'h00);
    end
`endif
endmodule
