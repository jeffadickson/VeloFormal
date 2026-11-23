// Register Check fixture: clean 32-bit CSR bank with RW (masked), RO, W1C, W1S, and WO registers.
module csr_bank (
    input  logic        clk,
    input  logic        rst_n,
    input  logic        wr_en,
    input  logic [3:0]  addr,
    input  logic [31:0] wdata,
    output logic [31:0] rdata
);
    logic [7:0]  ctrl_reg;   // 0x0: RW, mask 0x000000FF, reset 0x00000000
    logic [31:0] status_reg; // 0x4: RO, reset 0x000000A5
    logic [7:0]  irq_reg;    // 0x8: W1C, mask 0x000000FF, reset 0x000000FF
    logic [7:0]  en_reg;     // 0xC: W1S, mask 0x000000FF, reset 0x00000000
    logic [31:0] kick_reg;   // 0xE: WO, reads as 0

    initial begin
        ctrl_reg   = 8'h00;
        status_reg = 32'h0000_00A5;
        irq_reg    = 8'hFF;
        en_reg     = 8'h00;
        kick_reg   = 32'h0;
    end

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            ctrl_reg   <= 8'h00;
            status_reg <= 32'h0000_00A5;
            irq_reg    <= 8'hFF;
            en_reg     <= 8'h00;
            kick_reg   <= 32'h0;
        end else if (wr_en) begin
            case (addr)
                4'h0: ctrl_reg <= wdata[7:0];
                4'h4: ; // RO: ignore writes
                4'h8: irq_reg  <= irq_reg & ~wdata[7:0]; // W1C
                4'hC: en_reg   <= en_reg | wdata[7:0];   // W1S
                4'hE: kick_reg <= wdata;                 // WO
                default: ;
            endcase
        end
    end

    always_comb begin
        case (addr)
            4'h0: rdata = {24'h0, ctrl_reg};
            4'h4: rdata = status_reg;
            4'h8: rdata = {24'h0, irq_reg};
            4'hC: rdata = {24'h0, en_reg};
            4'hE: rdata = 32'h0; // WO reads as 0
            default: rdata = 32'h0;
        endcase
    end
endmodule
