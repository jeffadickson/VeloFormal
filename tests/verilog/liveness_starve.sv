module liveness_starve (
    input  logic clk,
    input  logic req,
    input  logic stall,
    output logic ack
);
    logic pending;
    initial pending = 1'b0;

    // Bug: if `stall` stays high forever, `ack` never fires!
    assign ack = pending && !stall;

    always_ff @(posedge clk) begin
        if (req)
            pending <= 1'b1;
        else if (ack)
            pending <= 1'b0;
    end

`ifdef FORMAL
    // Under fairness that `req` occurs infinitely often, `ack` should eventually fire.
    // Without fairness on `!stall`, a fair starving lasso exists where `stall == 1` forever!
    assume property (@(posedge clk) s_eventually req);
    assert property (@(posedge clk) s_eventually ack);
`endif
endmodule
