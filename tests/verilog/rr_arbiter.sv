module rr_arbiter (
    input  logic       clk,
    input  logic       rst_n,
    input  logic [1:0] req,
    output logic [1:0] gnt
);
    logic priority_bit; // 0: req[0] has priority, 1: req[1] has priority

    initial priority_bit = 1'b0;

    always_comb begin
        gnt = 2'b00;
        if (!priority_bit) begin
            if (req[0])      gnt = 2'b01;
            else if (req[1]) gnt = 2'b10;
        end else begin
            if (req[1])      gnt = 2'b10;
            else if (req[0]) gnt = 2'b01;
        end
    end

    always_ff @(posedge clk) begin
        if (!rst_n)
            priority_bit <= 1'b0;
        else if (gnt[0])
            priority_bit <= 1'b1;
        else if (gnt[1])
            priority_bit <= 1'b0;
    end

`ifdef FORMAL
    always_comb begin
        // Mutual exclusion: at most one grant active
        assert ($onehot0(gnt));
        // Only grant to active requesters
        assert ((gnt & ~req) == 2'b00);
        // Work conserving: if any request is active, someone gets granted
        if (|req)
            assert (|gnt);
        cover (gnt == 2'b01);
        cover (gnt == 2'b10);
    end

    always_ff @(posedge clk) begin
        // Round-robin fairness: if both request on consecutive cycles, grant alternates
        if (!$initstate && $past(rst_n) && $past(req == 2'b11) && req == 2'b11)
            assert (gnt != $past(gnt));
    end
`endif
endmodule
