// Example 10a: Packet Header Parser with 4 classic RTL bugs (caught by `veloformal autocheck`!)
// ZERO assertions needed — AutoCheck catches OOB LUT index, div-by-zero, incomplete unique case,
// and an FSM error deadlock state automatically.
module packet_parser_buggy (
    input  logic       clk,
    input  logic       pkt_valid,
    input  logic [2:0] proto_id,
    input  logic [7:0] pkt_len,
    input  logic [7:0] mtu,
    input  logic [1:0] hdr_type,
    output logic [7:0] weight,
    output logic [7:0] frag_cnt,
    output logic [7:0] hdr_bytes,
    output logic [1:0] parse_state
);
    // BUG 1: 5-entry LUT indexed by 3-bit proto_id (5..7 are out-of-bounds!)
    logic [7:0] proto_lut [0:4];
    assign weight = proto_lut[proto_id];

    // BUG 2: Division by zero when mtu == 0!
    assign frag_cnt = pkt_len / mtu;

    // BUG 3: Incomplete `unique case` missing hdr_type == 2'd3!
    always_comb begin
        hdr_bytes = 8'd0;
        unique case (hdr_type)
            2'd0: hdr_bytes = 8'd20;
            2'd1: hdr_bytes = 8'd40;
            2'd2: hdr_bytes = 8'd8;
        endcase
    end

    // BUG 4: FSM error state 2'd3 is a permanent deadlock trap!
    initial parse_state = 2'd0;
    always_ff @(posedge clk) begin
        case (parse_state)
            2'd0: parse_state <= pkt_valid ? 2'd1 : 2'd0;
            2'd1: parse_state <= (hdr_type == 2'd3) ? 2'd3 : 2'd2;
            2'd2: parse_state <= 2'd0;
            2'd3: parse_state <= 2'd3; // Deadlock!
        endcase
    end
endmodule
