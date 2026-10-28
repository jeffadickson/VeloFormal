module counter (
    input logic clk,
    input logic rst_n,
    output logic [7:0] count
);
    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= 8'd0;
        else
            count <= count + 8'd1;
    end

`ifdef FORMAL
    // Intentionally fails once the free-running counter reaches two. This is
    // the Stage-1 negative differential fixture, not a design requirement.
    always_ff @(posedge clk)
        assert (count < 8'd2);
`endif
endmodule
