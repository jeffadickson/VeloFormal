module counter_prove (
    input  logic       clk,
    input  logic       rst_n,
    input  logic       en,
    output logic [3:0] count
);
    initial count = 4'd0;

    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= 4'd0;
        else if (en) begin
            if (count == 4'd9)
                count <= 4'd0;
            else
                count <= count + 4'd1;
        end
    end

`ifdef FORMAL
    always_comb begin
        assert (count < 4'd10);
        cover (count == 4'd9);
    end
`endif
endmodule
