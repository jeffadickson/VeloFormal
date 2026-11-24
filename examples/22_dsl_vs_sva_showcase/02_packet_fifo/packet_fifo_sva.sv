// Tier 2 — Traditional IEEE 1800 SVA `bind` + Manual Symbolic Scoreboard (~104 lines!)
// Pain points:
//  1. Must hand-roll the infamous 60-line `$anyconst` shadow-pointer scoreboard state machine
//     just to prove FIFO data integrity (one off-by-one bug invalidates the entire proof!).
//  2. Must manually duplicate shadow `f_wr_ptr` / `f_rd_ptr` / `f_first_in` / `f_second_in`.
//  3. Must repeat `@(posedge clk) disable iff (rst)` and port bindings everywhere.
module packet_fifo_sva (
    input logic       clk,
    input logic       rst,
    input logic       wr_en,
    input logic [7:0] wr_data,
    input logic       rd_en,
    input logic [7:0] rd_data,
    input logic       full,
    input logic       empty,
    input logic [2:0] level
);
    // Boilerplate #1: Manual symbolic constants for 2-packet ordering & data integrity
    (* anyconst *) logic [2:0] f_const_addr;
    (* anyconst *) logic [7:0] f_const_data;

    // Boilerplate #2: Duplicate shadow FIFO pointers & tracking FSM
    logic [2:0] f_wr_ptr, f_rd_ptr;
    logic       f_in_fifo;

    wire f_do_wr = wr_en && !full;
    wire f_do_rd = rd_en && !empty;

    initial begin
        f_wr_ptr  = 3'd0;
        f_rd_ptr  = 3'd0;
        f_in_fifo = 1'b0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            f_wr_ptr  <= 3'd0;
            f_rd_ptr  <= 3'd0;
            f_in_fifo <= 1'b0;
        end else begin
            if (f_do_wr) begin
                if ((f_wr_ptr == f_const_addr) && (wr_data == f_const_data))
                    f_in_fifo <= 1'b1;
                f_wr_ptr <= f_wr_ptr + 3'd1;
            end
            if (f_do_rd) begin
                if (f_rd_ptr == f_const_addr)
                    f_in_fifo <= 1'b0;
                f_rd_ptr <= f_rd_ptr + 3'd1;
            end
        end
    end

    // Boilerplate #3: Shadow level & flag consistency invariants
    wire [2:0] f_fill = f_wr_ptr - f_rd_ptr;

    a_not_full_and_empty: assert property (
        @(posedge clk) disable iff (rst) !(full && empty)
    );

    a_level_bounded: assert property (
        @(posedge clk) disable iff (rst) level <= 3'd4
    );

    a_flags_match_level: assert property (
        @(posedge clk) disable iff (rst)
        (empty == (level == 3'd0)) && (full == (level == 3'd4))
    );

    a_shadow_fill_matches_rtl: assert property (
        @(posedge clk) disable iff (rst) level == f_fill
    );

    // Boilerplate #4: When our tracked symbolic packet reaches the head, rd_data MUST match!
    a_data_integrity: assert property (
        @(posedge clk) disable iff (rst)
        (f_in_fifo && !empty && (f_rd_ptr == f_const_addr)) |-> (rd_data == f_const_data)
    );

    // Boilerplate #5: Reachability covers
    c_reaches_full: cover property (
        @(posedge clk) disable iff (rst) full
    );

    c_tracks_symbolic_pop: cover property (
        @(posedge clk) disable iff (rst)
        f_in_fifo && f_do_rd && (f_rd_ptr == f_const_addr)
    );
endmodule

bind packet_fifo packet_fifo_sva u_packet_fifo_sva (
    .clk(clk), .rst(rst),
    .wr_en(wr_en), .wr_data(wr_data),
    .rd_en(rd_en), .rd_data(rd_data),
    .full(full), .empty(empty), .level(level)
);
