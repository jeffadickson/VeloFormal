// ISO 26262 Functional Safety Example 1: Triple Modular Redundancy (TMR) Voter
// Any single-bit SA0, SA1, or transient SEU fault in r0, r1, or r2 is 100% MASKED!
// Run: veloformal fault examples/21_questa_parity/tmr_voter.sv --target r0 --target r1 --target r2
module tmr_voter (
    input  logic       clk,
    input  logic       rst,
    input  logic [3:0] din,
    output logic [3:0] safe_out,
    output logic       tmr_err
);
    logic [3:0] r0, r1, r2;
    initial begin
        r0 = 4'd0;
        r1 = 4'd0;
        r2 = 4'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            r0 <= 4'd0;
            r1 <= 4'd0;
            r2 <= 4'd0;
        end else begin
            r0 <= din;
            r1 <= din;
            r2 <= din;
        end
    end

    // Bitwise majority voter masks any single-copy fault.
    assign safe_out = (r0 & r1) | (r1 & r2) | (r0 & r2);
    assign tmr_err  = (r0 != r1) || (r1 != r2);
endmodule
