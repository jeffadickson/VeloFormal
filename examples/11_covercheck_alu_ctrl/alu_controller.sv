// Example 11: ALU Controller for CoverCheck Dead-Code Unreachability Analysis
// `mode_cnt` cycles through 0..3, so defensive branches `mode_cnt == 5` and `mode_cnt == 6`
// are formally proved UNREACHABLE (safe coverage waivers!) by `veloformal covercheck`.
module alu_controller (
    input  logic       clk,
    input  logic       step,
    input  logic [7:0] op_a,
    input  logic [7:0] op_b,
    output logic [2:0] mode_cnt,
    output logic [7:0] result
);
    initial mode_cnt = 3'd0;

    always_ff @(posedge clk) begin
        if (step) begin
            if (mode_cnt == 3'd3)
                mode_cnt <= 3'd0;
            else
                mode_cnt <= mode_cnt + 3'd1;
        end
    end

    always_comb begin
        if (mode_cnt == 3'd0) begin
            result = op_a + op_b;
        end else if (mode_cnt == 3'd1) begin
            result = op_a - op_b;
        end else if (mode_cnt == 3'd2) begin
            result = op_a & op_b;
        end else if (mode_cnt == 3'd3) begin
            result = op_a ^ op_b;
        end else if (mode_cnt == 3'd5) begin
            // Unreachable dead code — proved unreachable by CoverCheck!
            result = 8'hDE;
        end else begin
            // Also unreachable because mode_cnt is always in 0..3!
            result = 8'hAD;
        end
    end
endmodule
