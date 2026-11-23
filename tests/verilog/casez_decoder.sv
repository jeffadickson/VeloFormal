module casez_decoder (
    input  logic [3:0] req,
    output logic [1:0] idx,
    output logic       valid
);
    always_comb begin
        valid = 1'b1;
        casez (req)
            4'b1???: idx = 2'd3;
            4'b01??: idx = 2'd2;
            4'b001?: idx = 2'd1;
            4'b0001: idx = 2'd0;
            default: begin
                idx   = 2'd0;
                valid = 1'b0;
            end
        endcase
    end

`ifdef FORMAL
    always_comb begin
        assert (valid == (|req));
        if (valid)
            assert (req[idx] == 1'b1);
        if (req[3])
            assert (idx == 2'd3);
    end
`endif
endmodule
