// CDC / RDC + Gray-code test fixture with two clock domains (clk_a, clk_b).
module cdc_design (
    input  logic       clk_a,
    input  logic       clk_b,
    input  logic       arst_a,
    input  logic       din_a,
    input  logic       inc_a,
    output logic       synced_out,
    output logic       unsync_out,
    output logic [3:0] multibit_out,
    output logic [3:0] gray_cnt,
    output logic [3:0] bin_cnt
);
    // Domain A registers
    logic src_safe;
    logic src_unsafe;
    logic src_arst;
    logic [3:0] bus_a;

    initial begin
        src_safe   = 1'b0;
        src_unsafe = 1'b0;
        src_arst   = 1'b0;
        bus_a      = 4'd0;
        bin_cnt    = 4'd0;
        gray_cnt   = 4'd0;
    end

    always_ff @(posedge clk_a) begin
        src_safe   <= din_a;
        src_unsafe <= din_a;
        bus_a      <= bus_a + 4'd1;
        if (inc_a) begin
            bin_cnt  <= bin_cnt + 4'd1;
            gray_cnt <= (bin_cnt + 4'd1) ^ ((bin_cnt + 4'd1) >> 1);
        end
    end

    // Async-reset FF in domain A -> feeds non-async-reset FF (RDC crossing!)
    always_ff @(posedge clk_a or posedge arst_a) begin
        if (arst_a)
            src_arst <= 1'b0;
        else
            src_arst <= din_a;
    end

    logic rdc_dst;
    initial rdc_dst = 1'b0;
    always_ff @(posedge clk_a) begin
        rdc_dst <= src_arst;
    end

    // Domain B: proper 2-FF synchronizer for `src_safe`
    logic sync1, sync2;
    initial begin
        sync1        = 1'b0;
        sync2        = 1'b0;
        unsync_out   = 1'b0;
        multibit_out = 4'd0;
    end

    always_ff @(posedge clk_b) begin
        sync1        <= src_safe;
        sync2        <= sync1;
        // Unsynchronized 1-bit CDC crossing (used directly as output without stage 2!)
        unsync_out   <= src_unsafe;
        // Unsynchronized multi-bit CDC crossing!
        multibit_out <= bus_a;
    end

    assign synced_out = sync2 | rdc_dst;
endmodule
