module velosim_bit_implicit_enable_register(
    input bit clk,
    input bit enable,
    input bit [7:0] data,
    output bit [7:0] q);
    always_ff @(posedge clk)
        if (enable)
            q <= data;
endmodule
