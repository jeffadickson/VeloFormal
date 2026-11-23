// CoverCheck fixture: has reachable branches AND an unreachable dead branch (cnt >= 4'd10
// is impossible because cnt wraps at 4'd7).
module covercheck_dead (
    input  logic       clk,
    input  logic       en,
    output logic [3:0] cnt,
    output logic       flag
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

    always_comb begin
        if (cnt == 4'd3) begin
            flag = 1'b1;
        end else if (cnt == 4'd12) begin
            // Unreachable dead code!
            flag = 1'b1;
        end else begin
            flag = 1'b0;
        end
    end
endmodule
