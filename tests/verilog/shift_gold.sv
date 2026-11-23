module shift_gold (
    input  logic       clk,
    input  logic       rst_n,
    input  logic [7:0] din,
    output logic [7:0] dout
);
    logic [7:0] s0, s1;
    initial begin
        s0 = 8'd0;
        s1 = 8'd0;
    end
    always_ff @(posedge clk) begin
        if (!rst_n) begin
            s0 <= 8'd0;
            s1 <= 8'd0;
        end else begin
            s0 <= din;
            s1 <= s0;
        end
    end
    assign dout = s1;
endmodule
