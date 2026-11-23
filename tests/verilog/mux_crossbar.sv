// Connectivity Check fixture: static wire, conditional mux, inverted signal, and 1-cycle pipeline.
module mux_crossbar (
    input  logic       clk,
    input  logic [1:0] sel,
    input  logic [7:0] in_a,
    input  logic [7:0] in_b,
    input  logic [7:0] in_c,
    output logic [7:0] out_mux,
    output logic [7:0] out_inv,
    output logic [7:0] out_pipe
);
    always_comb begin
        case (sel)
            2'd0: out_mux = in_a;
            2'd1: out_mux = in_b;
            default: out_mux = in_c;
        endcase
    end

    assign out_inv = ~in_a;

    always_ff @(posedge clk) begin
        out_pipe <= in_b;
    end
endmodule
