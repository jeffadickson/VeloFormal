module shift_gate (
    input  logic       clk,
    input  logic       rst_n,
    input  logic [7:0] din,
    output logic [7:0] dout
);
    // Packed 16-bit shift pipeline instead of two separate 8-bit registers
    logic [15:0] pipe;
    initial pipe = 16'd0;
    always_ff @(posedge clk) begin
        if (!rst_n)
            pipe <= 16'd0;
        else
            pipe <= {pipe[7:0], din};
    end
    assign dout = pipe[15:8];
endmodule
