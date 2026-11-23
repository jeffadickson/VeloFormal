// Register Check buggy fixture: RO register is accidentally writable and aliases CTRL!
module csr_bank_buggy (
    input  logic        clk,
    input  logic        rst_n,
    input  logic        wr_en,
    input  logic [3:0]  addr,
    input  logic [31:0] wdata,
    output logic [31:0] rdata
);
    logic [7:0]  ctrl_reg;
    logic [31:0] status_reg;

    initial begin
        ctrl_reg   = 8'h00;
        status_reg = 32'h0000_00A5;
    end

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            ctrl_reg   <= 8'h00;
            status_reg <= 32'h0000_00A5;
        end else if (wr_en) begin
            case (addr)
                4'h0: ctrl_reg <= wdata[7:0];
                4'h4: begin
                    // BUG 1: RO register is modified!
                    status_reg <= wdata;
                    // BUG 2: Aliases ctrl_reg!
                    ctrl_reg <= wdata[7:0];
                end
                default: ;
            endcase
        end
    end

    always_comb begin
        case (addr)
            4'h0: rdata = {24'h0, ctrl_reg};
            4'h4: rdata = status_reg;
            default: rdata = 32'h0;
        endcase
    end
endmodule
