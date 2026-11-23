module adder_gate (
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] sum
);
    // Bitwise ripple-carry adder: structurally different, functionally identical.
    logic [8:0] c;
    assign c[0] = 1'b0;
    genvar i;
    generate
        for (i = 0; i < 8; i = i + 1) begin : gen_fa
            assign sum[i] = a[i] ^ b[i] ^ c[i];
            assign c[i+1] = (a[i] & b[i]) | (a[i] & c[i]) | (b[i] & c[i]);
        end
    endgenerate
endmodule
