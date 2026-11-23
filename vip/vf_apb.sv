// VeloFormal Formal Assertion Library (VIP): AMBA APB Protocol Checker
// Verifies SETUP -> ACCESS phase transitions and control/data stability during wait states.
module vf_apb #(
    parameter int ADDR_W = 16,
    parameter int DATA_W = 32
) (
    input logic              pclk,
    input logic              presetn,
    input logic              psel,
    input logic              penable,
    input logic              pwrite,
    input logic [ADDR_W-1:0] paddr,
    input logic [DATA_W-1:0] pwdata,
    input logic              pready,
    input logic [DATA_W-1:0] prdata
);
    logic              past_valid;
    logic              prev_psel;
    logic              prev_penable;
    logic              prev_pwrite;
    logic [ADDR_W-1:0] prev_paddr;
    logic [DATA_W-1:0] prev_pwdata;
    logic              prev_pready;

    initial begin
        past_valid   = 1'b0;
        prev_psel    = 1'b0;
        prev_penable = 1'b0;
        prev_pwrite  = 1'b0;
        prev_paddr   = '0;
        prev_pwdata  = '0;
        prev_pready  = 1'b0;
    end

    always_ff @(posedge pclk) begin
        if (!presetn) begin
            past_valid   <= 1'b0;
            prev_psel    <= 1'b0;
            prev_penable <= 1'b0;
            prev_pwrite  <= 1'b0;
            prev_paddr   <= '0;
            prev_pwdata  <= '0;
            prev_pready  <= 1'b0;
        end else begin
            past_valid   <= 1'b1;
            prev_psel    <= psel;
            prev_penable <= penable;
            prev_pwrite  <= pwrite;
            prev_paddr   <= paddr;
            prev_pwdata  <= pwdata;
            prev_pready  <= pready;
        end
    end

`ifdef FORMAL
    always_comb begin
        if (presetn) begin
            // PENABLE must never be asserted without PSEL.
            assert (!penable || psel);

            if (past_valid) begin
                // SETUP phase (psel && !penable) must transition to ACCESS phase (psel && penable)
                // with stable address, direction, and write data.
                if (prev_psel && !prev_penable) begin
                    assert (psel && penable);
                    assert (paddr == prev_paddr);
                    assert (pwrite == prev_pwrite);
                    if (prev_pwrite) assert (pwdata == prev_pwdata);
                end

                // During ACCESS wait state (psel && penable && !pready), all transfer signals stay stable.
                if (prev_psel && prev_penable && !prev_pready) begin
                    assert (psel && penable);
                    assert (paddr == prev_paddr);
                    assert (pwrite == prev_pwrite);
                    if (prev_pwrite) assert (pwdata == prev_pwdata);
                end
            end

            cover (psel && penable && pready && pwrite);
            cover (psel && penable && pready && !pwrite);
        end
    end
`endif
endmodule
