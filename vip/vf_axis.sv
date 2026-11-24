// VeloFormal Formal Assertion Library (VIP): AMBA AXI4-Stream Protocol Checker
// Verifies tvalid/tready stability of tdata, tkeep, and tlast under backpressure + packet covers.
module vf_axis #(
    parameter int DATA_W = 32
) (
    input logic                aclk,
    input logic                aresetn,
    input logic                tvalid,
    input logic                tready,
    input logic [DATA_W-1:0]   tdata,
    input logic [DATA_W/8-1:0] tkeep,
    input logic                tlast
);
    logic rst;
    assign rst = !aresetn;

    vf_handshake #(.WIDTH(DATA_W + DATA_W/8 + 1)) u_axis_hs (
        .clk(aclk),
        .rst(rst),
        .valid(tvalid),
        .ready(tready),
        .data({tdata, tkeep, tlast})
    );

`ifdef FORMAL
    always_comb begin
        if (aresetn) begin
            c_axis_pkt_end: cover (tvalid && tready && tlast);
        end
    end
`endif
endmodule
