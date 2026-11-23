module for_loop_popcount (
    input  logic [7:0] x,
    output logic [3:0] cnt
);
    always_comb begin
        cnt = 4'd0;
        for (int i = 0; i < 8; i = i + 1) begin
            if (x[i])
                cnt = cnt + 4'd1;
        end
    end

`ifdef FORMAL
    always_comb begin
        assert (cnt == $countones(x));
        assert (cnt <= 4'd8);
    end
`endif
endmodule
