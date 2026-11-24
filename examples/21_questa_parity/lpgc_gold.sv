// Questa SLEC Low-Power Clock Gating (LPGC) Reference (Gold) — ungated datapath register
module lpgc_pipe (
    input  logic       clk,
    input  logic       rst,
    input  logic       in_valid,
    input  logic [7:0] in_data,
    output logic       out_valid,
    output logic [7:0] out_data
);
    always_ff @(posedge clk) begin
        if (rst) begin
            out_valid <= 1'b0;
            out_data  <= 8'd0;
        end else begin
            out_valid <= in_valid;
            out_data  <= in_data + 8'd1;
        end
    end
endmodule
