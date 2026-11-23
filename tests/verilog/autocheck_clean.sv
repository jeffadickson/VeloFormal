// AutoCheck clean RTL fixture: all checks (OOB, div-by-zero, unique case, FSM deadlock) must be SAFE.
module autocheck_clean (
    input  logic       clk,
    input  logic       rst,
    input  logic [2:0] idx,
    input  logic [7:0] a,
    input  logic [7:0] b,
    input  logic [1:0] sel,
    input  logic       go,
    output logic [7:0] mem_out,
    output logic [7:0] div_out,
    output logic [7:0] case_out,
    output logic [1:0] fsm_state
);
    logic [7:0] mem [0:5];
    assign mem_out = (idx < 3'd6) ? mem[idx] : 8'd0;
    assign div_out = (b != 8'd0) ? (a / b) : 8'd0;

    always_comb begin
        unique case (sel)
            2'd0: case_out = 8'd10;
            2'd1: case_out = 8'd20;
            2'd2: case_out = 8'd30;
            2'd3: case_out = 8'd40;
        endcase
    end

    initial fsm_state = 2'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            fsm_state <= 2'd0;
        end else begin
            case (fsm_state)
                2'd0: fsm_state <= go ? 2'd1 : 2'd0;
                2'd1: fsm_state <= go ? 2'd2 : 2'd0;
                2'd2: fsm_state <= 2'd0;
                default: fsm_state <= 2'd0;
            endcase
        end
    end
endmodule
