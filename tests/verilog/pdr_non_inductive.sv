// Classic non-inductive invariant: k-induction CANNOT prove `a != 4'd15` for any k,
// because from an arbitrary unreachable state where `a != b`, `a` becomes 15 immediately.
// IC3/PDR automatically discovers the inductive invariant `a == b` and proves `a != 15`!
module pdr_non_inductive (
    input  logic       clk,
    input  logic       en,
    output logic [3:0] a,
    output logic [3:0] b
);
    initial begin
        a = 4'd0;
        b = 4'd0;
    end

    always_ff @(posedge clk) begin
        if (a != b) begin
            a <= 4'd15;
        end else if (en) begin
            if (a == 4'd10) begin
                a <= 4'd0;
                b <= 4'd0;
            end else begin
                a <= a + 4'd1;
                b <= b + 4'd1;
            end
        end
    end

`ifdef FORMAL
    always_comb begin
        assert (a != 4'd15);
    end
`endif
endmodule
