// Example 10b: Fixed Packet Header Parser — proved 100% SAFE by `veloformal autocheck`!
module packet_parser_fixed (
    input  logic       clk,
    input  logic       rst,
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
    logic [7:0] proto_lut [0:4];
    assign weight   = (proto_id < 3'd5) ? proto_lut[proto_id] : 8'd1;
    assign frag_cnt = (mtu != 8'd0)     ? (pkt_len / mtu)     : 8'd0;

    always_comb begin
        unique case (hdr_type)
            2'd0: hdr_bytes = 8'd20;
            2'd1: hdr_bytes = 8'd40;
            2'd2: hdr_bytes = 8'd8;
            2'd3: hdr_bytes = 8'd16;
        endcase
    end

    initial parse_state = 2'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            parse_state <= 2'd0;
        end else begin
            case (parse_state)
                2'd0: parse_state <= pkt_valid ? 2'd1 : 2'd0;
                2'd1: parse_state <= (hdr_type == 2'd3) ? 2'd3 : 2'd2;
                2'd2: parse_state <= 2'd0;
                2'd3: parse_state <= 2'd0; // Auto-recover from error state!
            endcase
        end
    end
endmodule
