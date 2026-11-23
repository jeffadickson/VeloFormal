// Formal VIP testbench: compliant ready/valid producer & APB master checked by vip/ checkers.
module vip_dut_test (
    input  logic       clk,
    input  logic       rst,
    input  logic       req_valid,
    input  logic [7:0] req_data,
    input  logic       out_ready,
    input  logic       apb_start,
    input  logic       apb_wr,
    input  logic [7:0] apb_addr,
    input  logic [7:0] apb_wdata,
    input  logic       apb_pready
);
    // 1. Registered valid/ready pipeline stage (guaranteed compliant with vf_handshake!)
    logic       out_valid;
    logic [7:0] out_data;
    initial begin
        out_valid = 1'b0;
        out_data  = 8'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            out_valid <= 1'b0;
            out_data  <= 8'd0;
        end else if (!out_valid || out_ready) begin
            out_valid <= req_valid;
            out_data  <= req_data;
        end
    end

    vf_handshake #(.WIDTH(8)) u_chk_hs (
        .clk(clk),
        .rst(rst),
        .valid(out_valid),
        .ready(out_ready),
        .data(out_data)
    );

    // 2. Simple APB master FSM (IDLE -> SETUP -> ACCESS) checked by vf_apb!
    logic [1:0] apb_state; // 0=IDLE, 1=SETUP, 2=ACCESS
    logic       psel, penable, pwrite;
    logic [7:0] paddr, pwdata;

    initial begin
        apb_state = 2'd0;
        pwrite    = 1'b0;
        paddr     = 8'd0;
        pwdata    = 8'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            apb_state <= 2'd0;
            pwrite    <= 1'b0;
            paddr     <= 8'd0;
            pwdata    <= 8'd0;
        end else begin
            case (apb_state)
                2'd0: if (apb_start) begin
                    apb_state <= 2'd1;
                    pwrite    <= apb_wr;
                    paddr     <= apb_addr;
                    pwdata    <= apb_wdata;
                end
                2'd1: apb_state <= 2'd2;
                2'd2: if (apb_pready) apb_state <= 2'd0;
                default: apb_state <= 2'd0;
            endcase
        end
    end

    assign psel    = (apb_state == 2'd1) || (apb_state == 2'd2);
    assign penable = (apb_state == 2'd2);

    vf_apb #(.ADDR_W(8), .DATA_W(8)) u_chk_apb (
        .pclk(clk),
        .presetn(!rst),
        .psel(psel),
        .penable(penable),
        .pwrite(pwrite),
        .paddr(paddr),
        .pwdata(pwdata),
        .pready(apb_pready),
        .prdata(8'd0)
    );
endmodule
