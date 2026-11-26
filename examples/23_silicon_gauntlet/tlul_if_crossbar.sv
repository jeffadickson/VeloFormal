// SystemVerilog interface + modport TileLink-UL style register bridge
interface tlul_if;
    logic        a_valid;
    logic        a_ready;
    logic        a_write;
    logic [7:0]  a_data;
    logic        d_valid;
    logic [7:0]  d_data;

    modport host (
        output a_valid, a_write, a_data,
        input  a_ready, d_valid, d_data
    );
    modport dev (
        input  a_valid, a_write, a_data,
        output a_ready, d_valid, d_data
    );
endinterface

module tlul_csr_dev (
    input  logic clk,
    input  logic rst,
    tlul_if.dev  tl
);
    logic [7:0] csr_reg = 8'd0;
    logic       rsp_v   = 1'b0;
    logic [7:0] rsp_d   = 8'd0;

    assign tl.a_ready = 1'b1;
    assign tl.d_valid = rsp_v;
    assign tl.d_data  = rsp_d;

    always_ff @(posedge clk) begin
        if (rst) begin
            csr_reg <= 8'd0;
            rsp_v   <= 1'b0;
            rsp_d   <= 8'd0;
        end else begin
            rsp_v <= tl.a_valid && tl.a_ready;
            if (tl.a_valid && tl.a_ready) begin
                if (tl.a_write) begin
                    csr_reg <= tl.a_data;
                    rsp_d   <= tl.a_data;
                end else begin
                    rsp_d   <= csr_reg;
                end
            end
        end
    end
endmodule

module tlul_if_crossbar (
    input  logic       clk,
    input  logic       rst,
    input  logic       req_valid_i,
    input  logic       req_write_i,
    input  logic [7:0] req_data_i,
    output logic       req_ready_o,
    output logic       rsp_valid_o,
    output logic [7:0] rsp_data_o
);
    tlul_if u_tl();

    assign u_tl.a_valid = req_valid_i;
    assign u_tl.a_write = req_write_i;
    assign u_tl.a_data  = req_data_i;
    assign req_ready_o  = u_tl.a_ready;
    assign rsp_valid_o  = u_tl.d_valid;
    assign rsp_data_o   = u_tl.d_data;

    tlul_csr_dev u_dev (
        .clk (clk),
        .rst (rst),
        .tl  (u_tl.dev)
    );
endmodule
