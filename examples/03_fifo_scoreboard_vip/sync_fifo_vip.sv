// Example 03: Parameterized Synchronous FIFO verified via vip/vf_fifo_props.sv
// Instantiates the reusable Formal VIP FIFO scoreboard ($anyconst tracked packet) to prove
// zero overflow, zero underflow, flag consistency, and end-to-end data integrity.
module sync_fifo_vip #(
    parameter int WIDTH = 8,
    parameter int DEPTH = 4
) (
    input  logic             clk,
    input  logic             rst,
    input  logic             wr_en,
    input  logic [WIDTH-1:0] wr_data,
    input  logic             rd_en,
    output logic [WIDTH-1:0] rd_data,
    output logic             full,
    output logic             empty
);
    localparam int PTR_W = $clog2(DEPTH);

    logic [WIDTH-1:0] mem [0:DEPTH-1];
    logic [PTR_W:0]   wr_ptr;
    logic [PTR_W:0]   rd_ptr;

    initial begin
        wr_ptr = '0;
        rd_ptr = '0;
    end

    assign empty   = (wr_ptr == rd_ptr);
    assign full    = (wr_ptr[PTR_W] != rd_ptr[PTR_W]) &&
                     (wr_ptr[PTR_W-1:0] == rd_ptr[PTR_W-1:0]);
    assign rd_data = mem[rd_ptr[PTR_W-1:0]];

    wire do_wr = wr_en && !full;
    wire do_rd = rd_en && !empty;

    always_ff @(posedge clk) begin
        if (rst) begin
            wr_ptr <= '0;
            rd_ptr <= '0;
        end else begin
            if (do_wr) begin
                mem[wr_ptr[PTR_W-1:0]] <= wr_data;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (do_rd) begin
                rd_ptr <= rd_ptr + 1'b1;
            end
        end
    end

`ifdef FORMAL
    vf_fifo_props #(.WIDTH(WIDTH), .DEPTH(DEPTH)) u_fifo_vip (
        .clk(clk),
        .rst(rst),
        .wr_en(wr_en),
        .wr_data(wr_data),
        .rd_en(rd_en),
        .rd_data(rd_data),
        .full(full),
        .empty(empty)
    );
`endif
endmodule
