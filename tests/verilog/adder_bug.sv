module adder_bug (
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] sum
);
    // Subtle bug: XORs bit 7 when a == 8'h55 and b == 8'h2a
    assign sum = (a == 8'h55 && b == 8'h2a) ? ((a + b) ^ 8'h80) : (a + b);
endmodule
