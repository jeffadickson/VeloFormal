// OpenTitan-style PPC Arbiter + Streaming Endianness + InstanceArray + case inside
module lane_filter (
    input  logic [7:0] raw_i,
    output logic [7:0] clean_o
);
    always_comb begin
        case (raw_i) inside
            [8'h00:8'h7F]: clean_o = raw_i;
            default:       clean_o = 8'h7F;
        endcase
    end
endmodule

module ot_prim_arbiter_ppc (
    input  logic            clk,
    input  logic            rst,
    input  logic [3:0]      req_i,
    input  logic [3:0][7:0] data_i,
    input  logic [15:0]     be_word_i,
    output logic [3:0]      gnt_o,
    output logic            valid_o,
    output logic [7:0]      data_o,
    output logic [15:0]     le_word_o,
    output logic [15:0]     roundtrip_word_o
);
    // InstanceArray of 4 lane filters
    logic [3:0][7:0] filtered;
    lane_filter u_lanes [4] (
        .raw_i   (data_i),
        .clean_o (filtered)
    );

    // Streaming byte-swap ({<<8{...}}) and double-swap roundtrip
    assign le_word_o        = {<<8{be_word_i}};
    assign roundtrip_word_o = {<<8{le_word_o}};

    // Generate-for priority grant calculation
    logic [3:0] masked_req;
    assign masked_req[0] = req_i[0];
    genvar i;
    generate
        for (i = 1; i < 4; i++) begin : gen_prio
            assign masked_req[i] = req_i[i] && !(|req_i[i-1:0]);
        end
    endgenerate

    assign gnt_o   = masked_req;
    assign valid_o = |gnt_o;

    always_comb begin
        data_o = 8'd0;
        if (gnt_o[0])      data_o = filtered[0];
        else if (gnt_o[1]) data_o = filtered[1];
        else if (gnt_o[2]) data_o = filtered[2];
        else if (gnt_o[3]) data_o = filtered[3];
    end
endmodule
