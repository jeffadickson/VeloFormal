// VeloFormal Formal Assertion Library (VIP): Ready/Valid Handshake Checker
// Equivalent to Questa QVL / Formal Protocol Monitor for streaming valid/ready interfaces.
module vf_handshake #(
    parameter int WIDTH = 8
) (
    input logic             clk,
    input logic             rst,
    input logic             valid,
    input logic             ready,
    input logic [WIDTH-1:0] data
);
    logic             past_valid;
    logic             prev_valid;
    logic             prev_ready;
    logic [WIDTH-1:0] prev_data;

    initial begin
        past_valid = 1'b0;
        prev_valid = 1'b0;
        prev_ready = 1'b0;
        prev_data  = '0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            past_valid <= 1'b0;
            prev_valid <= 1'b0;
            prev_ready <= 1'b0;
            prev_data  <= '0;
        end else begin
            past_valid <= 1'b1;
            prev_valid <= valid;
            prev_ready <= ready;
            prev_data  <= data;
        end
    end

`ifdef FORMAL
    always_comb begin
        if (!rst && past_valid && prev_valid && !prev_ready) begin
            // Once asserted, valid must remain high until ready is sampled high.
            assert (valid);
            // Payload must remain stable while stalled.
            assert (data == prev_data);
        end
        if (!rst) begin
            // Cover an active transfer and a backpressure stall.
            cover (valid && ready);
            cover (valid && !ready);
        end
    end
`endif
endmodule
