// VeloFormal Formal Assertion Library (VIP): Wishbone B4 Pipelined/Classic Protocol Checker
// Verifies stb -> cyc, request stability under stall, and ack only during active cycle.
module vf_wishbone #(
    parameter int ADDR_W = 32,
    parameter int DATA_W = 32
) (
    input logic                clk,
    input logic                rst,
    input logic                cyc,
    input logic                stb,
    input logic                we,
    input logic [ADDR_W-1:0]   adr,
    input logic [DATA_W-1:0]   dat_o,
    input logic [DATA_W/8-1:0] sel,
    input logic                stall,
    input logic                ack,
    input logic [DATA_W-1:0]   dat_i
);
    logic                past_valid;
    logic                prev_cyc, prev_stb, prev_we, prev_stall;
    logic [ADDR_W-1:0]   prev_adr;
    logic [DATA_W-1:0]   prev_dat;
    logic [DATA_W/8-1:0] prev_sel;

    initial begin
        past_valid = 1'b0;
        prev_cyc   = 1'b0;
        prev_stb   = 1'b0;
        prev_we    = 1'b0;
        prev_stall = 1'b0;
        prev_adr   = '0;
        prev_dat   = '0;
        prev_sel   = '0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            past_valid <= 1'b0;
            prev_cyc   <= 1'b0;
            prev_stb   <= 1'b0;
            prev_we    <= 1'b0;
            prev_stall <= 1'b0;
            prev_adr   <= '0;
            prev_dat   <= '0;
            prev_sel   <= '0;
        end else begin
            past_valid <= 1'b1;
            prev_cyc   <= cyc;
            prev_stb   <= stb;
            prev_we    <= we;
            prev_stall <= stall;
            prev_adr   <= adr;
            prev_dat   <= dat_o;
            prev_sel   <= sel;
        end
    end

`ifdef FORMAL
    always_comb begin
        if (!rst) begin
            // STB must never be asserted without CYC
            a_wb_stb_cyc: assert (!stb || cyc);
            // ACK must only be asserted during an active CYC
            a_wb_ack_cyc: assert (!ack || cyc);

            // When stalled (cyc && stb && stall), master must hold cyc, stb, we, adr, sel, dat_o stable
            if (past_valid && prev_cyc && prev_stb && prev_stall) begin
                a_wb_stall_hold: assert (cyc && stb && we == prev_we && adr == prev_adr && sel == prev_sel);
                if (prev_we) a_wb_stall_wdata: assert (dat_o == prev_dat);
            end

            c_wb_write_ack: cover (cyc && ack && we);
            c_wb_read_ack:  cover (cyc && ack && !we);
        end
    end
`endif
endmodule
