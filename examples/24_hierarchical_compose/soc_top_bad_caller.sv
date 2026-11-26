// Buggy top-level SoC: forgets to clamp req_a / req_b before driving u_alu!
// Hierarchical assume-guarantee composition MUST catch this caller precondition violation.
module soc_top_bad_caller (
    input  logic        clk,
    input  logic        rst,
    input  logic        req_valid,
    input  logic [7:0]  req_a,
    input  logic [7:0]  req_b,
    output logic        soc_valid,
    output logic [15:0] soc_data
);
    logic        alu_valid;
    logic [15:0] alu_val;

    heavy_alu_leaf u_alu (
        .clk       (clk),
        .rst       (rst),
        .in_valid  (req_valid),
        .in_a      (req_a),
        .in_b      (req_b),
        .out_valid (alu_valid),
        .out_val   (alu_val)
    );

    initial begin
        soc_valid = 1'b0;
        soc_data  = 16'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            soc_valid <= 1'b0;
            soc_data  <= 16'd0;
        end else begin
            soc_valid <= alu_valid;
            soc_data  <= alu_valid ? alu_val : 16'd0;
        end
    end
endmodule
