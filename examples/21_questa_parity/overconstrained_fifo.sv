// Questa PropCheck Over-Constraint / Dead-End Detector Showcase
// The environment assumptions force `push == 1` every cycle AND `count < 4`,
// which is satisfiable for cycles 0..3, then hits a DEAD-END at cycle 4 where
// NO input assignment can satisfy the assumptions!
// Run: veloformal deadend examples/21_questa_parity/overconstrained_fifo.sv
module overconstrained_fifo (
    input  logic       clk,
    input  logic       push,
    output logic [2:0] count
);
    initial count = 3'd0;

    always_ff @(posedge clk) begin
        if (push) count <= count + 3'd1;
    end

    // Over-constrained assumptions: forcing push=1 drives count to 4, violating count < 4!
    assume property (@(posedge clk) push == 1'b1);
    assume property (@(posedge clk) count < 3'd4);

    // Because of the dead-end at cycle 4, this bogus assertion would look "safe" beyond cycle 4!
    assert property (@(posedge clk) count < 3'd5);
endmodule
