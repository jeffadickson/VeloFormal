module counter_init (
    input logic clk,
    input logic rst_n,
    output logic [7:0] count
);
    // This explicit initialization is a Stage-1 differential-fixture feature.
    // It is not inferred from the reset branch by the VeloHdl adapter.
    initial count = 8'd0;

    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= 8'd0;
        else
            count <= count + 8'd1;
    end

`ifdef FORMAL
    // Align the external environment with the VeloFormal contract's require.
    always @* assume (rst_n);
    always_ff @(posedge clk)
        assert (count < 8'd2);
`endif
endmodule
