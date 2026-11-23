// Example 06b: Fixed-Priority Arbiter Starvation Bug (caught by `veloformal live`!)
// Master 0 always beats Master 1, so under `req[0] && req[1]`, Master 1 starves forever on a lasso.
module fixed_prio_starve (
    input  logic       clk,
    input  logic       rst,
    input  logic [1:0] req,
    output logic [1:0] gnt
);
    assign gnt = req[0] ? 2'b01 : (req[1] ? 2'b10 : 2'b00);

`ifdef FORMAL
    always_comb begin
        assume (!rst);
        a_mutex: assert ($onehot0(gnt));
    end

    // Liveness check: if master 1 requests, it should eventually be granted — FAILS with lasso!
    assert property (@(posedge clk) s_eventually (!req[1] || gnt[1]));
`endif
endmodule
