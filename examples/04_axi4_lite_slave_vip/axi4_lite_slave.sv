// Example 04: AMBA AXI4-Lite Slave Register verified with vip/vf_axi4_lite.sv
// Proves all 5 AXI4-Lite channels obey stability-under-backpressure and read-after-write integrity.
module axi4_lite_slave #(
    parameter int ADDR_W = 8,
    parameter int DATA_W = 32
) (
    input  logic                aclk,
    input  logic                aresetn,
    // Write Address
    input  logic                awvalid,
    output logic                awready,
    input  logic [ADDR_W-1:0]   awaddr,
    // Write Data
    input  logic                wvalid,
    output logic                wready,
    input  logic [DATA_W-1:0]   wdata,
    input  logic [DATA_W/8-1:0] wstrb,
    // Write Response
    output logic                bvalid,
    input  logic                bready,
    output logic [1:0]          bresp,
    // Read Address
    input  logic                arvalid,
    output logic                arready,
    input  logic [ADDR_W-1:0]   araddr,
    // Read Data
    output logic                rvalid,
    input  logic                rready,
    output logic [DATA_W-1:0]   rdata,
    output logic [1:0]          rresp
);
    logic [DATA_W-1:0] reg_val;

    initial begin
        reg_val = '0;
        bvalid  = 1'b0;
        bresp   = 2'b00;
        rvalid  = 1'b0;
        rdata   = '0;
        rresp   = 2'b00;
    end

    // Accept write when AW & W are both valid and no response is pending
    wire do_wr = awvalid && wvalid && !bvalid;
    assign awready = do_wr;
    assign wready  = do_wr;

    // Accept read when AR is valid and no read data is pending
    wire do_rd = arvalid && !rvalid;
    assign arready = do_rd;

    always_ff @(posedge aclk) begin
        if (!aresetn) begin
            reg_val <= '0;
            bvalid  <= 1'b0;
            bresp   <= 2'b00;
            rvalid  <= 1'b0;
            rdata   <= '0;
            rresp   <= 2'b00;
        end else begin
            if (do_wr) begin
                reg_val <= wdata;
                bvalid  <= 1'b1;
                bresp   <= 2'b00; // OKAY
            end else if (bvalid && bready) begin
                bvalid <= 1'b0;
            end

            if (do_rd) begin
                rdata  <= reg_val;
                rvalid <= 1'b1;
                rresp  <= 2'b00; // OKAY
            end else if (rvalid && rready) begin
                rvalid <= 1'b0;
            end
        end
    end

`ifdef FORMAL
    // Assume master obeys AXI4-Lite AW/W/AR stability under backpressure
    logic              past_valid;
    logic              prev_awv, prev_awr, prev_wv, prev_wr, prev_arv, prev_arr;
    logic [ADDR_W-1:0] prev_awaddr, prev_araddr;
    logic [DATA_W-1:0] prev_wdata;
    logic [DATA_W/8-1:0] prev_wstrb;

    initial begin
        past_valid  = 1'b0;
        prev_awv    = 1'b0;
        prev_awr    = 1'b0;
        prev_wv     = 1'b0;
        prev_wr     = 1'b0;
        prev_arv    = 1'b0;
        prev_arr    = 1'b0;
        prev_awaddr = '0;
        prev_araddr = '0;
        prev_wdata  = '0;
        prev_wstrb  = '0;
    end

    always_ff @(posedge aclk) begin
        if (!aresetn) begin
            past_valid <= 1'b0;
        end else begin
            past_valid  <= 1'b1;
            prev_awv    <= awvalid;
            prev_awr    <= awready;
            prev_awaddr <= awaddr;
            prev_wv     <= wvalid;
            prev_wr     <= wready;
            prev_wdata  <= wdata;
            prev_wstrb  <= wstrb;
            prev_arv    <= arvalid;
            prev_arr    <= arready;
            prev_araddr <= araddr;
        end
    end

    always_comb begin
        if (aresetn && past_valid) begin
            if (prev_awv && !prev_awr) assume (awvalid && awaddr == prev_awaddr);
            if (prev_wv  && !prev_wr)  assume (wvalid && wdata == prev_wdata && wstrb == prev_wstrb);
            if (prev_arv && !prev_arr) assume (arvalid && araddr == prev_araddr);
        end
    end

    // Instantiate 5-channel AXI4-Lite Formal VIP protocol monitor!
    vf_axi4_lite #(.ADDR_W(ADDR_W), .DATA_W(DATA_W)) u_axi_vip (
        .aclk(aclk), .aresetn(aresetn),
        .awvalid(awvalid), .awready(awready), .awaddr(awaddr),
        .wvalid(wvalid),   .wready(wready),   .wdata(wdata), .wstrb(wstrb),
        .bvalid(bvalid),   .bready(bready),   .bresp(bresp),
        .arvalid(arvalid), .arready(arready), .araddr(araddr),
        .rvalid(rvalid),   .rready(rready),   .rdata(rdata), .rresp(rresp)
    );
`endif
endmodule
