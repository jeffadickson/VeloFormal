module velosim_priority_enable_register(
    input bit clk,
    input bit first,
    input bit second,
    input bit [7:0] a,
    input bit [7:0] b,
    output bit [7:0] q);
    always_ff @(posedge clk)
        if (first)
            q <= a;
        else if (second)
            q <= b;
endmodule
