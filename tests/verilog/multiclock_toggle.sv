module multiclock_toggle (
    input  logic clk_a,
    input  logic clk_b,
    output logic a,
    output logic b
);
    initial begin
        a = 1'b0;
        b = 1'b0;
    end

    always_ff @(posedge clk_a) begin
        a <= ~a;
    end

    always_ff @(posedge clk_b) begin
        b <= ~b;
    end

`ifdef FORMAL
    // If both clocks stay identical every tick, both toggles must remain equal!
    always_comb begin
        assume (clk_a == clk_b);
        assert (a == b);
    end
`endif
endmodule
