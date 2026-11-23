module sync_fifo #(
    parameter int WIDTH = 8,
    parameter int DEPTH = 4
) (
    input  logic             clk,
    input  logic             rst_n,
    input  logic             wr_en,
    input  logic [WIDTH-1:0] wr_data,
    input  logic             rd_en,
    output logic [WIDTH-1:0] rd_data,
    output logic             full,
    output logic             empty,
    output logic [2:0]       count
);
    logic [WIDTH-1:0] mem [0:DEPTH-1];
    logic [1:0] wr_ptr;
    logic [1:0] rd_ptr;

    initial begin
        wr_ptr = 2'd0;
        rd_ptr = 2'd0;
        count  = 3'd0;
    end

    assign full    = (count == DEPTH[2:0]);
    assign empty   = (count == 3'd0);
    assign rd_data = mem[rd_ptr];

    logic do_wr;
    logic do_rd;
    assign do_wr = wr_en && !full;
    assign do_rd = rd_en && !empty;

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            wr_ptr <= 2'd0;
            rd_ptr <= 2'd0;
            count  <= 3'd0;
        end else begin
            if (do_wr) begin
                mem[wr_ptr] <= wr_data;
                wr_ptr <= wr_ptr + 2'd1;
            end
            if (do_rd) begin
                rd_ptr <= rd_ptr + 2'd1;
            end
            if (do_wr && !do_rd)
                count <= count + 3'd1;
            else if (!do_wr && do_rd)
                count <= count - 3'd1;
        end
    end

`ifdef FORMAL
    always_comb begin
        assert (count <= DEPTH[2:0]);
        assert (full == (count == DEPTH[2:0]));
        assert (empty == (count == 3'd0));
        assert (!(full && empty));
        cover (full);
    end

    always_ff @(posedge clk) begin
        if (!$initstate && $past(rst_n) && $past(do_wr) && !$past(do_rd))
            assert (count == $past(count) + 3'd1);
    end
`endif
endmodule
