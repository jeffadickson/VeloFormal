module signed_compare (
    input logic clk,
    input logic rst_n,
    output logic signed [7:0] count
);
    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= -8'sd1;
        else if (count < 8'sd0)
            count <= -count;
        else
            count <= count - 8'sd1;
    end

`ifdef FORMAL
    // Every signed eight-bit value is at most signed 127. This is the Stage-1
    // positive differential fixture paired with a VeloFormal invariant.
    always_ff @(posedge clk)
        assert (count <= 8'sd127);
`endif
endmodule
