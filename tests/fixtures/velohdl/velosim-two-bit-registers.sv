module velosim_two_bit_registers(
    input bit clk,
    input bit [7:0] data,
    output bit [7:0] q,
    output bit [7:0] r);
    always_ff @(posedge clk) begin
        q <= data;
        r <= q;
    end
endmodule
