module velosim_bit_memory_read_write(
    input bit clk,
    input bit write_enable,
    input bit [1:0] address,
    input bit [7:0] data,
    output bit [7:0] q);
    bit [7:0] memory [0:3];
    always_ff @(posedge clk) begin
        if (write_enable)
            memory[address] <= data;
        q <= memory[address];
    end
endmodule
