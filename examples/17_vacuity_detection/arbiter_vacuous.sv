// Example 17: PropCheck Vacuity Detection (`veloformal vacuity`)
// Demonstrates how an assertion can "pass" formal proof ONLY because its antecedent can never fire!
// Here `grant_count` saturates at 4'd7, so `a_real_check` is NON-VACUOUS while `a_impossible_guard`
// is formally proved VACUOUS!
module arbiter_vacuous (
    input  logic       clk,
    input  logic       rst,
    input  logic       req,
    output logic [3:0] grant_count
);
    initial grant_count = 4'd0;

    always_ff @(posedge clk) begin
        if (rst) begin
            grant_count <= 4'd0;
        end else if (req && grant_count < 4'd7) begin
            grant_count <= grant_count + 4'd1;
        end
    end

`ifdef FORMAL
    always_comb begin
        // NON-VACUOUS: antecedent `grant_count == 4'd4` fires at cycle 4
        if (grant_count == 4'd4) begin
            a_real_check: assert (grant_count <= 4'd7);
        end

        // VACUOUS: `grant_count` can never reach 4'd9 — `prove` says PROVED, but `vacuity` catches it!
        if (grant_count == 4'd9) begin
            a_impossible_guard: assert (grant_count == 4'd0);
        end
    end
`endif
endmodule
