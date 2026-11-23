// Example 14: DMA Channel Reset & X-Propagation Analysis (`veloformal xcheck`)
// `err_code` forgot to reset/initialize -> leaks silicon power-up X to `status_out` (X-LEAK)!
// `valid_out` is properly initialized and `masked_data` is gated by a controlling 0 when invalid (X-FREE!).
module dma_channel (
    input  logic       clk,
    input  logic       rst,
    input  logic       start,
    input  logic [7:0] burst_data,
    output logic       valid_out,
    output logic [7:0] masked_data,
    output logic [7:0] status_out
);
    logic       active;
    logic [7:0] data_buf;  // Uninitialized! Only safe when gated by `active`
    logic [7:0] err_code;  // BUG: missing reset/init -> leaks X to status_out!

    initial active = 1'b0;

    always_ff @(posedge clk) begin
        if (rst) begin
            active <= 1'b0;
        end else if (start) begin
            active   <= 1'b1;
            data_buf <= burst_data;
            err_code <= 8'h00;
        end else begin
            active <= 1'b0;
        end
    end

    assign valid_out   = active;
    // IEEE 1800 controlling 0: when `active == 0`, `{8{active}} == 8'h00`, so `data_buf` X is masked!
    // And when `active == 1`, `data_buf` was just loaded from known `burst_data` on the prior cycle!
    assign masked_data = active ? data_buf : 8'h00;
    // BUG: `err_code` is X before the first `start` pulse!
    assign status_out  = err_code;
endmodule
