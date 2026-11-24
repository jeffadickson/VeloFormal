// Tier 2 RTL: 4-Entry Synchronous Packet FIFO (Pure Production RTL — Zero `ifdef FORMAL)
module packet_fifo (
    input  logic       clk,
    input  logic       rst,
    input  logic       wr_en,
    input  logic [7:0] wr_data,
    input  logic       rd_en,
    output logic [7:0] rd_data,
    output logic       full,
    output logic       empty,
    output logic [2:0] level
);
    logic [7:0] mem [0:3];
    logic [2:0] wr_ptr, rd_ptr;

    assign level   = wr_ptr - rd_ptr;
    assign empty   = (wr_ptr == rd_ptr);
    assign full    = (level == 3'd4);
    assign rd_data = mem[rd_ptr[1:0]];

    wire do_wr = wr_en && !full;
    wire do_rd = rd_en && !empty;

    initial begin
        wr_ptr = 3'd0;
        rd_ptr = 3'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            wr_ptr <= 3'd0;
            rd_ptr <= 3'd0;
        end else begin
            if (do_wr) begin
                mem[wr_ptr[1:0]] <= wr_data;
                wr_ptr           <= wr_ptr + 3'd1;
            end
            if (do_rd) begin
                rd_ptr <= rd_ptr + 3'd1;
            end
        end
    end
endmodule
