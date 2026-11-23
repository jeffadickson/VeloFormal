module counter_fail (
    input  logic       clk,
    input  logic       rst_n,
    output logic [7:0] count
);
    initial count = 8'd0;

    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= 8'd0;
        else
            count <= count + 8'd1;
    end

`ifdef FORMAL
    always_comb begin
        assume (rst_n == 1'b1);
        assert (count < 8'd5);
    end
`endif
endmodule
