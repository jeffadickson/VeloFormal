module velosim_bit_reset_register(
    input bit clk,
    input bit rst_n,
    input bit [7:0] data,
    output bit [7:0] q);
    always_ff @(posedge clk)
        if (!rst_n)
            q <= 8'd0;
        else
            q <= data;
endmodule
