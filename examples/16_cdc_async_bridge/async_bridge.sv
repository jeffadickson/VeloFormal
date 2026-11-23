// Example 16a: Clean Dual-Clock Bridge (2-FF Synchronized Handshake + Formally Verified Gray Pointer)
// Passes `veloformal cdc --gray wr_ptr_gray` with zero errors!
module async_bridge (
    input  logic       wr_clk,
    input  logic       rd_clk,
    input  logic       wr_inc,
    input  logic       req_a,
    output logic       req_b,
    output logic [3:0] wr_ptr_gray,
    output logic [3:0] rd_ptr_gray_sync
);
    // Write clock domain
    logic       req_a_ff;
    logic [3:0] wr_ptr_bin;

    initial begin
        req_a_ff    = 1'b0;
        wr_ptr_bin  = 4'd0;
        wr_ptr_gray = 4'd0;
    end

    always_ff @(posedge wr_clk) begin
        req_a_ff <= req_a;
        if (wr_inc) begin
            wr_ptr_bin  <= wr_ptr_bin + 4'd1;
            wr_ptr_gray <= (wr_ptr_bin + 4'd1) ^ ((wr_ptr_bin + 4'd1) >> 1);
        end
    end

    // Read clock domain: 2-FF synchronizer for 1-bit req + 2-FF synchronizer for Gray pointer
    logic       req_sync1, req_sync2;
    logic [3:0] ptr_sync1, ptr_sync2;

    initial begin
        req_sync1 = 1'b0;
        req_sync2 = 1'b0;
        ptr_sync1 = 4'd0;
        ptr_sync2 = 4'd0;
    end

    always_ff @(posedge rd_clk) begin
        req_sync1 <= req_a_ff;
        req_sync2 <= req_sync1;
        ptr_sync1 <= wr_ptr_gray;
        ptr_sync2 <= ptr_sync1;
    end

    assign req_b            = req_sync2;
    assign rd_ptr_gray_sync = ptr_sync2;
endmodule
