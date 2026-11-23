// VeloFormal Formal Assertion Library (VIP): AMBA AXI4-Lite Protocol Checker
// Verifies channel stability under backpressure across AW, W, B, AR, and R channels.
module vf_axi4_lite #(
    parameter int ADDR_W = 32,
    parameter int DATA_W = 32
) (
    input logic                aclk,
    input logic                aresetn,
    // Write Address channel
    input logic                awvalid,
    input logic                awready,
    input logic [ADDR_W-1:0]   awaddr,
    // Write Data channel
    input logic                wvalid,
    input logic                wready,
    input logic [DATA_W-1:0]   wdata,
    input logic [DATA_W/8-1:0] wstrb,
    // Write Response channel
    input logic                bvalid,
    input logic                bready,
    input logic [1:0]          bresp,
    // Read Address channel
    input logic                arvalid,
    input logic                arready,
    input logic [ADDR_W-1:0]   araddr,
    // Read Data channel
    input logic                rvalid,
    input logic                rready,
    input logic [DATA_W-1:0]   rdata,
    input logic [1:0]          rresp
);
    logic rst;
    assign rst = !aresetn;

    vf_handshake #(.WIDTH(ADDR_W)) u_aw (
        .clk(aclk), .rst(rst), .valid(awvalid), .ready(awready), .data(awaddr)
    );
    vf_handshake #(.WIDTH(DATA_W + DATA_W/8)) u_w (
        .clk(aclk), .rst(rst), .valid(wvalid), .ready(wready), .data({wdata, wstrb})
    );
    vf_handshake #(.WIDTH(2)) u_b (
        .clk(aclk), .rst(rst), .valid(bvalid), .ready(bready), .data(bresp)
    );
    vf_handshake #(.WIDTH(ADDR_W)) u_ar (
        .clk(aclk), .rst(rst), .valid(arvalid), .ready(arready), .data(araddr)
    );
    vf_handshake #(.WIDTH(DATA_W + 2)) u_r (
        .clk(aclk), .rst(rst), .valid(rvalid), .ready(rready), .data({rdata, rresp})
    );
endmodule
