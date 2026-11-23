// Vacuity fixture: one non-vacuous assertion (cnt == 3) and one vacuous assertion (cnt == 12
// which is impossible because cnt wraps at 7).
module vacuity_test (
    input  logic       clk,
    input  logic       en,
    output logic [3:0] cnt
);
    initial cnt = 4'd0;

    always_ff @(posedge clk) begin
        if (en) begin
            if (cnt == 4'd7)
                cnt <= 4'd0;
            else
                cnt <= cnt + 4'd1;
        end
    end

`ifdef FORMAL
    always_comb begin
        // Non-vacuous: `cnt == 3` IS reachable (cycle 3) and `cnt < 8` holds.
        if (cnt == 4'd3) begin
            a_reachable: assert (cnt < 4'd8);
        end
        // Vacuous: `cnt == 12` is NEVER reachable, so this assertion passes vacuously!
        if (cnt == 4'd12) begin
            a_vacuous: assert (cnt == 4'd0);
        end
    end
`endif
endmodule
