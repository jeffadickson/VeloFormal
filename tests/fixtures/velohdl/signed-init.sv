module signed_init (
    input logic clk,
    input logic rst_n,
    output logic signed [7:0] count
);
    initial count = -8'sd1;

    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= -8'sd1;
        else if (count < 8'sd0)
            count <= -count;
        else
            count <= count - 8'sd1;
    end

`ifdef FORMAL
    always @* assume (rst_n);
    // This fails after the signed negative branch produces positive one.
    always_ff @(posedge clk)
        assert (count <= 8'sd0);
`endif
endmodule
