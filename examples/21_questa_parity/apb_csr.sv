// Questa Register Check Example: Native AMBA APB3 CSR Slave
// Verified directly via:
//   veloformal regcheck examples/21_questa_parity/apb_csr.sv --spec examples/21_questa_parity/apb_csr.rdl --protocol apb
module apb_csr (
    input  logic        pclk,
    input  logic        presetn,
    input  logic        psel,
    input  logic        penable,
    input  logic        pwrite,
    input  logic [3:0]  paddr,
    input  logic [7:0]  pwdata,
    output logic [7:0]  prdata,
    output logic        pready
);
    assign pready = 1'b1;

    logic [7:0] ctrl_reg;   // 0x0: RW, mask 0x0F, reset 0x05
    logic [7:0] stat_reg;   // 0x4: RO, reset 0xA5
    logic [7:0] irq_reg;    // 0x8: W1C, mask 0xFF, reset 0x0F
    logic [7:0] key_reg;    // 0xC: WO, reads 0x00

    wire apb_wr = psel && penable && pwrite && pready;
    wire apb_rd = psel && penable && !pwrite && pready;

    always_ff @(posedge pclk) begin
        if (!presetn) begin
            ctrl_reg <= 8'h05;
            stat_reg <= 8'hA5;
            irq_reg  <= 8'h0F;
            key_reg  <= 8'h00;
        end else if (apb_wr) begin
            case (paddr)
                4'h0: ctrl_reg <= (pwdata & 8'h0F);
                4'h8: irq_reg  <= irq_reg & ~pwdata;
                4'hC: key_reg  <= pwdata;
                default: ;
            endcase
        end
    end

    always_comb begin
        prdata = 8'h00;
        if (apb_rd) begin
            case (paddr)
                4'h0: prdata = ctrl_reg;
                4'h4: prdata = stat_reg;
                4'h8: prdata = irq_reg;
                4'hC: prdata = 8'h00; // WO returns 0
                default: prdata = 8'h00;
            endcase
        end
    end
endmodule
