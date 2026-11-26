// OpenTitan-style package + parameterized sync FIFO exercising:
// - SystemVerilog packages & `import`
// - `typedef enum` & `typedef struct packed`
// - Structured assignment patterns `'{default: '0}` & `'{valid: ..., payload: ...}`
// - Multi-dimensional packed array storage `logic [DEPTH-1:0][WIDTH-1:0]`
// - Compound assignments (`+=`, `-=`) and `inside` operator
package ot_fifo_pkg;
    typedef enum logic [1:0] {
        FIFO_EMPTY = 2'd0,
        FIFO_MID   = 2'd1,
        FIFO_FULL  = 2'd2
    } fifo_level_e;

    typedef struct packed {
        logic       err;
        logic [7:0] data;
    } fifo_pkt_t;
endpackage

module ot_prim_fifo_sync #(
    parameter int DEPTH = 4
) (
    input  logic                    clk,
    input  logic                    rst,
    input  logic                    wvalid_i,
    output logic                    wready_o,
    input  ot_fifo_pkg::fifo_pkt_t  wdata_i,
    output logic                    rvalid_o,
    input  logic                    rready_i,
    output ot_fifo_pkg::fifo_pkt_t  rdata_o,
    output logic [2:0]              depth_o,
    output ot_fifo_pkg::fifo_level_e level_o
);
    import ot_fifo_pkg::*;

    logic [DEPTH-1:0][8:0] storage = '0;
    logic [1:0]            wptr    = 2'd0;
    logic [1:0]            rptr    = 2'd0;
    logic [2:0]            cnt     = 3'd0;

    assign wready_o = (cnt < DEPTH[2:0]);
    assign rvalid_o = (cnt > 3'd0);
    assign depth_o  = cnt;
    assign rdata_o  = rvalid_o ? fifo_pkt_t'(storage[rptr]) : '{default: '0};

    always_comb begin
        if (cnt == 3'd0)
            level_o = FIFO_EMPTY;
        else if (cnt inside {[3'd1:3'd3]})
            level_o = FIFO_MID;
        else
            level_o = FIFO_FULL;
    end

    wire do_push = wvalid_i && wready_o;
    wire do_pop  = rvalid_o && rready_i;

    always_ff @(posedge clk) begin
        if (rst) begin
            wptr    <= 2'd0;
            rptr    <= 2'd0;
            cnt     <= 3'd0;
            storage <= '0;
        end else begin
            if (do_push) begin
                storage[wptr] <= wdata_i;
                wptr += 2'd1;
            end
            if (do_pop) begin
                rptr += 2'd1;
            end
            if (do_push && !do_pop) begin
                cnt += 3'd1;
            end else if (!do_push && do_pop) begin
                cnt -= 3'd1;
            end
        end
    end
endmodule
