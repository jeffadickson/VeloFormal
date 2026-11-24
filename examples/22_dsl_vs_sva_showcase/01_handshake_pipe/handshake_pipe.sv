// Tier 1 RTL: Ready/Valid 1-Stage Pipelined Incrementer (Pure Production RTL — Zero `ifdef FORMAL)
module handshake_pipe (
    input  logic       clk,
    input  logic       rst,
    input  logic       in_valid,
    output logic       in_ready,
    input  logic [7:0] in_data,
    output logic       out_valid,
    input  logic       out_ready,
    output logic [7:0] out_data
);
    assign in_ready = !out_valid || out_ready;

    initial begin
        out_valid = 1'b0;
        out_data  = 8'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            out_valid <= 1'b0;
            out_data  <= 8'd0;
        end else if (in_ready) begin
            out_valid <= in_valid;
            if (in_valid) begin
                out_data <= in_data + 8'd1;
            end
        end
    end
endmodule
