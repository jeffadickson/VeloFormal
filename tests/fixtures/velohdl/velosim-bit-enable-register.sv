// Two-state enable fixture emitted by the pinned VeloSim structural exporter.
// Both branches write q so the formal bridge can preserve an explicit next mux.
module velosim_bit_enable_register(
    input bit clk,
    input bit enable,
    input bit [7:0] data,
    output bit [7:0] q
);
    always_ff @(posedge clk) begin
        if (enable) q <= data;
        else q <= q;
    end
endmodule
