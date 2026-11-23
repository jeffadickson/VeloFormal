// Example 07: UART Transmitter with Concurrent SVA Sequences (|->, |=>, ##N, disable iff)
// Proves IDLE line high, start-bit low on cycle 1 after launch, stop-bit high on cycle 10,
// and covers a complete 10-bit 8N1 UART frame transmission.
module uart_tx (
    input  logic       clk,
    input  logic       rst,
    input  logic       tx_start,
    input  logic [7:0] tx_data,
    output logic       tx_line,
    output logic       tx_busy,
    output logic       tx_done
);
    logic [9:0] shift_reg; // {stop(1), data[7:0], start(0)}
    logic [3:0] bit_cnt;   // 0 = idle, 1..10 = transmitting

    initial begin
        shift_reg = 10'h3FF;
        bit_cnt   = 4'd0;
        tx_done   = 1'b0;
    end

    assign tx_busy = (bit_cnt != 4'd0);
    assign tx_line = tx_busy ? shift_reg[0] : 1'b1;

    always_ff @(posedge clk) begin
        if (rst) begin
            shift_reg <= 10'h3FF;
            bit_cnt   <= 4'd0;
            tx_done   <= 1'b0;
        end else begin
            tx_done <= 1'b0;
            if (!tx_busy) begin
                if (tx_start) begin
                    shift_reg <= {1'b1, tx_data, 1'b0};
                    bit_cnt   <= 4'd10;
                end
            end else begin
                shift_reg <= {1'b1, shift_reg[9:1]};
                bit_cnt   <= bit_cnt - 4'd1;
                if (bit_cnt == 4'd1) begin
                    tx_done <= 1'b1;
                end
            end
        end
    end

`ifdef FORMAL
    // Idle line must always be high
    always_comb begin
        if (!tx_busy) a_idle_high: assert (tx_line == 1'b1);
        a_cnt_bounds: assert (bit_cnt <= 4'd10);
        c_tx_done:    cover (!rst && tx_done);
    end

    // Concurrent SVA: launching a frame drives start bit (0) on next cycle (`|=>`)!
    assert property (@(posedge clk) disable iff (rst) (!tx_busy && tx_start) |=> (tx_busy && tx_line == 1'b0));

    // Concurrent SVA: stop bit (1) appears 10 cycles after launch (`|-> ##10`)!
    assert property (@(posedge clk) disable iff (rst) (!tx_busy && tx_start) |-> ##10 (tx_busy && tx_line == 1'b1));
`endif
endmodule
