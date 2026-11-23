module sva_delay (
    input  logic clk,
    input  logic req,
    output logic ack
);
    logic d1, d2;
    initial begin
        d1 = 1'b0;
        d2 = 1'b0;
    end

    always_ff @(posedge clk) begin
        d1 <= req;
        d2 <= d1;
    end

    assign ack = d2;

`ifdef FORMAL
    assert property (@(posedge clk) req |-> ##2 ack);
`endif
endmodule
