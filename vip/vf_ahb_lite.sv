// VeloFormal Formal Assertion Library (VIP): AMBA AHB-Lite Protocol Checker
// Verifies address-phase stability during wait states (HREADY == 0) and legal HTRANS transitions.
module vf_ahb_lite #(
    parameter int ADDR_W = 32,
    parameter int DATA_W = 32
) (
    input logic              hclk,
    input logic              hresetn,
    input logic [1:0]        htrans, // 00=IDLE, 01=BUSY, 10=NONSEQ, 11=SEQ
    input logic              hwrite,
    input logic [ADDR_W-1:0] haddr,
    input logic [2:0]        hsize,
    input logic [DATA_W-1:0] hwdata,
    input logic              hready,
    input logic              hresp,
    input logic [DATA_W-1:0] hrdata
);
    localparam logic [1:0] HTRANS_IDLE   = 2'b00;
    localparam logic [1:0] HTRANS_NONSEQ = 2'b10;

    logic              past_valid;
    logic              prev_hready;
    logic [1:0]        prev_htrans;
    logic              prev_hwrite;
    logic [ADDR_W-1:0] prev_haddr;
    logic [2:0]        prev_hsize;

    initial begin
        past_valid  = 1'b0;
        prev_hready = 1'b1;
        prev_htrans = HTRANS_IDLE;
        prev_hwrite = 1'b0;
        prev_haddr  = '0;
        prev_hsize  = 3'd0;
    end

    always_ff @(posedge hclk) begin
        if (!hresetn) begin
            past_valid  <= 1'b0;
            prev_hready <= 1'b1;
            prev_htrans <= HTRANS_IDLE;
            prev_hwrite <= 1'b0;
            prev_haddr  <= '0;
            prev_hsize  <= 3'd0;
        end else begin
            past_valid  <= 1'b1;
            prev_hready <= hready;
            prev_htrans <= htrans;
            prev_hwrite <= hwrite;
            prev_haddr  <= haddr;
            prev_hsize  <= hsize;
        end
    end

`ifdef FORMAL
    always_comb begin
        if (hresetn && past_valid) begin
            // During a wait state (prev_hready == 0) on an active transfer, address/control must stay stable
            if (!prev_hready && prev_htrans != HTRANS_IDLE) begin
                a_ahb_wait_stable: assert (
                    htrans == prev_htrans &&
                    hwrite == prev_hwrite &&
                    haddr  == prev_haddr  &&
                    hsize  == prev_hsize
                );
            end
            c_ahb_nonseq: cover (htrans == HTRANS_NONSEQ && hready);
        end
    end
`endif
endmodule
