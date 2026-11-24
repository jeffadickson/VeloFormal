// VeloFormal Formal Assertion Library (VIP): SiFive TileLink-UL (Uncached Lightweight) Checker
// Verifies Channel A (request) and Channel D (response) stability under backpressure + opcode legality.
module vf_tilelink_ul #(
    parameter int ADDR_W = 32,
    parameter int DATA_W = 32
) (
    input logic                clk,
    input logic                rst,
    // Channel A (Master -> Slave)
    input logic                a_valid,
    input logic                a_ready,
    input logic [2:0]          a_opcode, // 0=PutFull, 1=PutPartial, 4=Get
    input logic [ADDR_W-1:0]   a_address,
    input logic [DATA_W/8-1:0] a_mask,
    input logic [DATA_W-1:0]   a_data,
    // Channel D (Slave -> Master)
    input logic                d_valid,
    input logic                d_ready,
    input logic [2:0]          d_opcode, // 0=AccessAck, 1=AccessAckData
    input logic [DATA_W-1:0]   d_data
);
    vf_handshake #(.WIDTH(3 + ADDR_W + DATA_W/8 + DATA_W)) u_tl_a (
        .clk(clk), .rst(rst), .valid(a_valid), .ready(a_ready),
        .data({a_opcode, a_address, a_mask, a_data})
    );
    vf_handshake #(.WIDTH(3 + DATA_W)) u_tl_d (
        .clk(clk), .rst(rst), .valid(d_valid), .ready(d_ready),
        .data({d_opcode, d_data})
    );

`ifdef FORMAL
    always_comb begin
        if (!rst) begin
            // Legal TL-UL Channel A opcodes: PutFullData(0), PutPartialData(1), Get(4)
            if (a_valid) begin
                a_tl_a_opcode: assert (a_opcode == 3'd0 || a_opcode == 3'd1 || a_opcode == 3'd4);
            end
            // Legal TL-UL Channel D opcodes: AccessAck(0), AccessAckData(1)
            if (d_valid) begin
                a_tl_d_opcode: assert (d_opcode == 3'd0 || d_opcode == 3'd1);
            end
        end
    end
`endif
endmodule
