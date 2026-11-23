// Example 15: UART Peripheral CSR Register Bank verified by `veloformal regcheck`
// Covers RW (with reserved bitmask), RO (hardware version ID), W1C (IRQ status),
// W1S (IRQ enable), WO (TX command kick), and cross-register non-aliasing.
module uart_csr (
    input  logic        clk,
    input  logic        rst_n,
    input  logic        wr_en,
    input  logic [7:0]  addr,
    input  logic [31:0] wdata,
    output logic [31:0] rdata
);
    logic [7:0]  ctrl;       // 0x00: RW, mask 0x000000FF, reset 0x00000001
    logic [31:0] version;    // 0x04: RO, reset 0x00010203
    logic [3:0]  irq_status; // 0x08: W1C, mask 0x0000000F, reset 0x0000000F
    logic [3:0]  irq_enable; // 0x0C: W1S, mask 0x0000000F, reset 0x00000000
    logic [31:0] tx_kick;    // 0x10: WO, reads as 0
    logic [15:0] baud_div;   // 0x14: RW, mask 0x0000FFFF, reset 0x0000006C

    initial begin
        ctrl       = 8'h01;
        version    = 32'h0001_0203;
        irq_status = 4'hF;
        irq_enable = 4'h0;
        tx_kick    = 32'h0;
        baud_div   = 16'h006C;
    end

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            ctrl       <= 8'h01;
            version    <= 32'h0001_0203;
            irq_status <= 4'hF;
            irq_enable <= 4'h0;
            tx_kick    <= 32'h0;
            baud_div   <= 16'h006C;
        end else if (wr_en) begin
            case (addr)
                8'h00: ctrl       <= wdata[7:0];
                8'h04: ; // RO version register ignores software writes
                8'h08: irq_status <= irq_status & ~wdata[3:0]; // W1C
                8'h0C: irq_enable <= irq_enable | wdata[3:0];  // W1S
                8'h10: tx_kick    <= wdata;                    // WO
                8'h14: baud_div   <= wdata[15:0];              // RW
                default: ;
            endcase
        end
    end

    always_comb begin
        case (addr)
            8'h00: rdata = {24'h0, ctrl};
            8'h04: rdata = version;
            8'h08: rdata = {28'h0, irq_status};
            8'h0C: rdata = {28'h0, irq_enable};
            8'h10: rdata = 32'h0; // WO always reads 0
            8'h14: rdata = {16'h0, baud_div};
            default: rdata = 32'h0;
        endcase
    end
endmodule
