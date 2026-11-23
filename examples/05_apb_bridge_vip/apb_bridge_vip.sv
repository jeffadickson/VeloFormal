// Example 05: AMBA APB Master Bridge verified with vip/vf_apb.sv
// Translates simple single-cycle requests into compliant 2-phase APB SETUP -> ACCESS transfers.
module apb_bridge_vip #(
    parameter int ADDR_W = 16,
    parameter int DATA_W = 32
) (
    input  logic              pclk,
    input  logic              presetn,
    // Simple request interface
    input  logic              req_valid,
    output logic              req_ready,
    input  logic              req_write,
    input  logic [ADDR_W-1:0] req_addr,
    input  logic [DATA_W-1:0] req_wdata,
    // APB master bus
    output logic              psel,
    output logic              penable,
    output logic              pwrite,
    output logic [ADDR_W-1:0] paddr,
    output logic [DATA_W-1:0] pwdata,
    input  logic              pready,
    input  logic [DATA_W-1:0] prdata
);
    localparam logic [1:0] ST_IDLE   = 2'd0;
    localparam logic [1:0] ST_SETUP  = 2'd1;
    localparam logic [1:0] ST_ACCESS = 2'd2;

    logic [1:0] state;

    initial begin
        state  = ST_IDLE;
        pwrite = 1'b0;
        paddr  = '0;
        pwdata = '0;
    end

    assign req_ready = (state == ST_IDLE);
    assign psel      = (state == ST_SETUP) || (state == ST_ACCESS);
    assign penable   = (state == ST_ACCESS);

    always_ff @(posedge pclk) begin
        if (!presetn) begin
            state  <= ST_IDLE;
            pwrite <= 1'b0;
            paddr  <= '0;
            pwdata <= '0;
        end else begin
            case (state)
                ST_IDLE: begin
                    if (req_valid) begin
                        state  <= ST_SETUP;
                        pwrite <= req_write;
                        paddr  <= req_addr;
                        pwdata <= req_wdata;
                    end
                end
                ST_SETUP: begin
                    state <= ST_ACCESS;
                end
                ST_ACCESS: begin
                    if (pready) state <= ST_IDLE;
                end
                default: state <= ST_IDLE;
            endcase
        end
    end

`ifdef FORMAL
    vf_apb #(.ADDR_W(ADDR_W), .DATA_W(DATA_W)) u_apb_vip (
        .pclk(pclk), .presetn(presetn),
        .psel(psel), .penable(penable), .pwrite(pwrite),
        .paddr(paddr), .pwdata(pwdata),
        .pready(pready), .prdata(prdata)
    );
`endif
endmodule
