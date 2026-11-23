// VeloFormal Formal Assertion Library (VIP): Reusable FIFO Safety & Data-Integrity Checker
// Uses a symbolic ($anyconst) tracked packet scoreboard to prove ordering & zero corruption
// for any synchronous FIFO of depth DEPTH.
module vf_fifo_props #(
    parameter int WIDTH = 8,
    parameter int DEPTH = 4
) (
    input logic             clk,
    input logic             rst,
    input logic             wr_en,
    input logic [WIDTH-1:0] wr_data,
    input logic             rd_en,
    input logic [WIDTH-1:0] rd_data,
    input logic             full,
    input logic             empty
);
    localparam int CNT_W = $clog2(DEPTH + 1);

    logic [CNT_W-1:0] count;
    wire do_wr = wr_en && !full;
    wire do_rd = rd_en && !empty;

    // Symbolic tracked item (`$anyconst`) — proves data integrity for ALL values & ALL slots!
    (* anyconst *) logic [WIDTH-1:0] tracked_val;
    logic [CNT_W-1:0] tracked_pos;
    logic             tracked_active;

    initial begin
        count          = '0;
        tracked_pos    = '0;
        tracked_active = 1'b0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            count          <= '0;
            tracked_pos    <= '0;
            tracked_active <= 1'b0;
        end else begin
            case ({do_wr, do_rd})
                2'b10: count <= count + 1'b1;
                2'b01: count <= count - 1'b1;
                default: ;
            endcase

            if (!tracked_active && do_wr && wr_data == tracked_val) begin
                tracked_active <= 1'b1;
                tracked_pos    <= do_rd ? (count - 1'b1) : count;
            end else if (tracked_active && do_rd) begin
                if (tracked_pos == '0) begin
                    tracked_active <= 1'b0;
                end else begin
                    tracked_pos <= tracked_pos - 1'b1;
                end
            end
        end
    end

`ifdef FORMAL
    always_comb begin
        if (!rst) begin
            // Count bounds and flag consistency
            assert (count <= DEPTH[CNT_W-1:0]);
            assert (empty == (count == '0));
            assert (full == (count == DEPTH[CNT_W-1:0]));
            assert (!(full && empty));

            // Data-integrity check: when the tracked item reaches head of FIFO and is read,
            // rd_data MUST match tracked_val!
            if (tracked_active && do_rd && tracked_pos == '0) begin
                assert (rd_data == tracked_val);
            end

            cover (full);
            cover (tracked_active && do_rd && tracked_pos == '0);
        end
    end
`endif
endmodule
