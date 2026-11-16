module velosim_parallel_enabled_registers(
    input bit clk,
    input bit enable,
    input bit [7:0] data,
    output bit [7:0] q,
    output bit [7:0] r);
    always_ff @(posedge clk)
        if (enable)
            q <= data;
    always_ff @(posedge clk)
        if (enable)
            r <= q;
endmodule
