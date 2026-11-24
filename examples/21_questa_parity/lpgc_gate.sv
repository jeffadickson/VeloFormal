// Questa SLEC Low-Power Clock Gating (LPGC) Implementation (Gate)
// Gates `out_data` FF when `!in_valid` (saves dynamic power) and adds an extra debug port `cg_active`.
// Equivalent to `lpgc_gold.sv` under `--reset rst=1 --valid-cond out_valid --ignore-output cg_active`!
module lpgc_pipe (
    input  logic       clk,
    input  logic       rst,
    input  logic       in_valid,
    input  logic [7:0] in_data,
    output logic       out_valid,
    output logic [7:0] out_data,
    output logic       cg_active
);
    assign cg_active = !in_valid;

    always_ff @(posedge clk) begin
        if (rst) begin
            out_valid <= 1'b0;
            out_data  <= 8'd0;
        end else begin
            out_valid <= in_valid;
            if (in_valid) begin
                out_data <= in_data + 8'd1;
            end
        end
    end
endmodule
