// Top-level SoC integrating heavy_alu_leaf with input sanitization and output stage.
module soc_top (
    input  logic        clk,
    input  logic        rst,
    input  logic        req_valid,
    input  logic [7:0]  req_a,
    input  logic [7:0]  req_b,
    output logic        soc_valid,
    output logic [15:0] soc_data
);
    // Clamp untrusted external bus operands to [0..127] to satisfy u_alu's precondition.
    logic [7:0] clamped_a;
    logic [7:0] clamped_b;
    assign clamped_a = (req_a > 8'd127) ? 8'd127 : req_a;
    assign clamped_b = (req_b > 8'd127) ? 8'd127 : req_b;

    logic        alu_valid;
    logic [15:0] alu_val;

    heavy_alu_leaf u_alu (
        .clk       (clk),
        .rst       (rst),
        .in_valid  (req_valid),
        .in_a      (clamped_a),
        .in_b      (clamped_b),
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
