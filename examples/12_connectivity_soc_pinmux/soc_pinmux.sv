// Example 12: SoC Pinmux & Interrupt Router verified via `veloformal connectivity`
// Demonstrates conditional mux routing, active-low inverted IRQ routing, and 1-cycle registered bypass.
module soc_pinmux (
    input  logic       clk,
    input  logic [1:0] func_sel,
    input  logic [7:0] gpio_out,
    input  logic [7:0] uart_out,
    input  logic [7:0] spi_out,
    input  logic       irq_in,
    input  logic       dbg_en,
    input  logic [7:0] dbg_bus,
    output logic [7:0] pad_out,
    output logic       irq_n_out,
    output logic [7:0] trace_out
);
    always_comb begin
        case (func_sel)
            2'd0: pad_out = gpio_out;
            2'd1: pad_out = uart_out;
            2'd2: pad_out = spi_out;
            default: pad_out = 8'h00;
        endcase
    end

    // Active-low inverted interrupt output
    assign irq_n_out = ~irq_in;

    // 1-cycle pipelined debug trace output when dbg_en is asserted
    initial trace_out = 8'h00;
    always_ff @(posedge clk) begin
        if (dbg_en)
            trace_out <= dbg_bus;
    end
endmodule
