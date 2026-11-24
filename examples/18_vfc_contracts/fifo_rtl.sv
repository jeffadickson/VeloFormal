// Pure production synchronous FIFO (ZERO formal code inside RTL!)
// Verified non-invasively using `examples/18_vfc_contracts/fifo_contract.vfc`.
module fifo_rtl #(
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
    logic [PTR_W:0]   wr_ptr, rd_ptr;

    initial begin
        wr_ptr = '0;
        rd_ptr = '0;
    end

    assign empty   = (wr_ptr == rd_ptr);
    assign full    = (wr_ptr[PTR_W] != rd_ptr[PTR_W]) &&
                     (wr_ptr[PTR_W-1:0] == rd_ptr[PTR_W-1:0]);
    assign rd_data = mem[rd_ptr[PTR_W-1:0]];

    always_ff @(posedge clk) begin
        if (rst) begin
            wr_ptr <= '0;
            rd_ptr <= '0;
        end else begin
            if (wr_en && !full) begin
                mem[wr_ptr[PTR_W-1:0]] <= wr_data;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (rd_en && !empty) begin
                rd_ptr <= rd_ptr + 1'b1;
            end
        end
    end
endmodule
