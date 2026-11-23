module alu_core (
    input  logic [1:0] op,
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] y
);
    always_comb begin
        case (op)
            2'd0: y = a + b;
            2'd1: y = a - b;
            2'd2: y = a & b;
            default: y = a ^ b;
        endcase
    end
endmodule

module alu_top (
    input  logic       clk,
    input  logic [1:0] op,
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] y_reg
);
    logic [7:0] y_comb;

    alu_core u_alu (
        .op (op),
        .a  (a),
        .b  (b),
        .y  (y_comb)
    );

    initial y_reg = 8'd0;

    always_ff @(posedge clk) begin
        y_reg <= y_comb;
    end

`ifdef FORMAL
    assert property (@(posedge clk) (op == 2'd3 && a == b) |=> (y_reg == 8'd0));
`endif
endmodule
