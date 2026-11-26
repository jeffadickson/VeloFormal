// Leaf submodule: 2-stage pipelined bounded adder/accumulator with internal state.
// In full-chip formal, blackboxing this block eliminates its internal datapath FFs
// from the top-level SoC proof while preserving exact assume-guarantee soundness.
module heavy_alu_leaf (
    input  logic        clk,
    input  logic        rst,
    input  logic        in_valid,
    input  logic [7:0]  in_a,
    input  logic [7:0]  in_b,
    output logic        out_valid,
    output logic [15:0] out_val
);
    logic        s1_valid    = 1'b0;
    logic [7:0]  s1_a        = 8'd0;
    logic [7:0]  s1_b        = 8'd0;
    logic [15:0] s1_sum      = 16'd0;
    logic [15:0] shadow_hist = 16'd0;

    initial begin
        out_valid = 1'b0;
        out_val   = 16'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            s1_valid    <= 1'b0;
            s1_a        <= 8'd0;
            s1_b        <= 8'd0;
            s1_sum      <= 16'd0;
            shadow_hist <= 16'd0;
            out_valid   <= 1'b0;
            out_val     <= 16'd0;
        end else begin
            // Stage 1: capture operands
            s1_valid    <= in_valid;
            s1_a        <= in_valid ? in_a : 8'd0;
            s1_b        <= in_valid ? in_b : 8'd0;
            s1_sum      <= {8'd0, in_a} + {8'd0, in_b};
            shadow_hist <= shadow_hist ^ {8'd0, in_a};

            // Stage 2: commit bounded result
            out_valid   <= s1_valid;
            out_val     <= s1_valid ? ({8'd0, s1_a} + {8'd0, s1_b}) : 16'd0;
        end
    end
endmodule
