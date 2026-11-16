module velosim_signed_less_than_enable_register(
    input bit clk,
    input bit signed [7:0] limit,
    input bit signed [7:0] data,
    output bit signed [7:0] q);
    always_ff @(posedge clk)
        if (q < limit)
            q <= data;
endmodule
