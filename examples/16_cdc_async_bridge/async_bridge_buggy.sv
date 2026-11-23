// Example 16b: Buggy Dual-Clock Bridge showing CDC, Reconvergence, and RDC violations!
// Run `veloformal cdc examples/16_cdc_async_bridge/async_bridge_buggy.sv` to inspect all findings.
module async_bridge_buggy (
    input  logic       wr_clk,
    input  logic       rd_clk,
    input  logic       wr_arst,
    input  logic       sig_a,
    input  logic       sig_b,
    input  logic [7:0] data_in,
    output logic       reconv_out,
    output logic       comb_cdc_out,
    output logic [7:0] bus_out
);
    logic       a_ff, b_ff, arst_ff;
    logic [7:0] bus_ff;

    initial begin
        a_ff    = 1'b0;
        b_ff    = 1'b0;
        arst_ff = 1'b0;
        bus_ff  = 8'd0;
    end

    always_ff @(posedge wr_clk) begin
        a_ff   <= sig_a;
        b_ff   <= sig_b;
        bus_ff <= data_in;
    end

    // Async-reset FF driving a non-async-reset FF in the same clock domain (RDC hazard!)
    always_ff @(posedge wr_clk or posedge wr_arst) begin
        if (wr_arst) arst_ff <= 1'b0;
        else         arst_ff <= sig_a;
    end

    logic rdc_capture;
    initial rdc_capture = 1'b0;
    always_ff @(posedge wr_clk) begin
        rdc_capture <= arst_ff;
    end

    // Two independently 2-FF synchronized signals that reconverge in rd_clk!
    logic a_s1, a_s2, b_s1, b_s2;
    initial begin
        a_s1         = 1'b0;
        a_s2         = 1'b0;
        b_s1         = 1'b0;
        b_s2         = 1'b0;
        reconv_out   = 1'b0;
        comb_cdc_out = 1'b0;
        bus_out      = 8'd0;
    end

    always_ff @(posedge rd_clk) begin
        a_s1 <= a_ff;
        a_s2 <= a_s1;
        b_s1 <= b_ff;
        b_s2 <= b_s1;

        // WARN: CDC reconvergence (a_s2 and b_s2 both originate from wr_clk!)
        reconv_out   <= a_s2 ^ b_s2;
        // ERROR: Combinational logic on CDC crossing path (glitch hazard!)
        comb_cdc_out <= a_ff & b_ff;
        // ERROR: Unsynchronized 8-bit multi-bit bus crossing!
        bus_out      <= bus_ff;
    end
endmodule
