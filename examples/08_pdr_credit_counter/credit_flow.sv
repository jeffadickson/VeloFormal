// Example 08: Credit-Based Flow Control (Non-Inductive Invariant proved by IC3/PDR!)
// Naive k-induction cannot prove `a_never_overflow` because an unreachable state can start with
// `used + free != 8`; ABC IC3/PDR automatically discovers the inductive invariant `used + free == 8`!
module credit_flow (
    input  logic       clk,
    input  logic       rst,
    input  logic       alloc_req,
    input  logic       release_req,
    output logic [3:0] credits_used,
    output logic [3:0] credits_free
);
    initial begin
        credits_used = 4'd0;
        credits_free = 4'd8;
    end

    wire do_alloc   = alloc_req   && (credits_free > 4'd0);
    wire do_release = release_req && (credits_used > 4'd0);

    always_ff @(posedge clk) begin
        if (rst) begin
            credits_used <= 4'd0;
            credits_free <= 4'd8;
        end else begin
            case ({do_alloc, do_release})
                2'b10: begin
                    credits_used <= credits_used + 4'd1;
                    credits_free <= credits_free - 4'd1;
                end
                2'b01: begin
                    credits_used <= credits_used - 4'd1;
                    credits_free <= credits_free + 4'd1;
                end
                default: ;
            endcase
        end
    end

`ifdef FORMAL
    always_comb begin
        // Property alone is NOT 1-step inductive (requires PDR to infer `used + free == 8`)!
        a_used_bound: assert (credits_used <= 4'd8);
        a_free_bound: assert (credits_free <= 4'd8);
        c_full_used:  cover (credits_used == 4'd8);
    end
`endif
endmodule
