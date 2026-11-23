module mem_rw (
    input  logic       clk,
    input  logic       we,
    input  logic [2:0] waddr,
    input  logic [7:0] wdata,
    input  logic [2:0] raddr,
    output logic [7:0] rdata
);
    logic [7:0] mem [0:7];

    always_ff @(posedge clk) begin
        if (we)
            mem[waddr] <= wdata;
    end

    assign rdata = mem[raddr];

`ifdef FORMAL
    // Pick an arbitrary symbolic address; if we wrote to it last cycle, reading it now returns that value.
    wire [2:0] f_addr = $anyconst;
    always_ff @(posedge clk) begin
        if (!$initstate && $past(we) && $past(waddr) == f_addr && raddr == f_addr)
            assert (rdata == $past(wdata));
    end
`endif
endmodule
