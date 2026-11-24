// Pure production RTL (ZERO `ifdef FORMAL` or SVA inside the RTL!)
// Verified non-invasively using `examples/18_vfc_contracts/counter.vfc`.
module counter_rtl (
    input  logic       clk,
    input  logic       rst,
    input  logic       enable,
    output logic [7:0] count
);
    initial count = 8'd0;

    always_ff @(posedge clk) begin
        if (rst) begin
            count <= 8'd0;
        end else if (enable) begin
            count <= count + 8'd1;
        end
    end
endmodule
