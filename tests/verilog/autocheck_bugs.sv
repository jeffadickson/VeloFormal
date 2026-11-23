// AutoCheck test fixture containing intentional RTL bugs (OOB memory, div-by-zero,
// unique-case violation, and FSM deadlock).
module autocheck_bugs (
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
    // Depth-6 memory indexed by 3-bit idx (values 6 and 7 are OOB!)
    logic [7:0] mem [0:5];
    assign mem_out = mem[idx];

    // Unguarded division by zero when b == 0
    assign div_out = a / b;

    // Incomplete unique case (sel == 2'd3 missing!)
    always_comb begin
        case_out = 8'd0;
        unique case (sel)
            2'd0: case_out = 8'd10;
            2'd1: case_out = 8'd20;
            2'd2: case_out = 8'd30;
        endcase
    end

    // FSM with deadlock state 2'd3 (once entered, never leaves even on rst=0!)
    // Wait: make sure deadlock holds for ALL inputs (including rst) so it's a true trap!
    initial fsm_state = 2'd0;
    always_ff @(posedge clk) begin
        case (fsm_state)
            2'd0: fsm_state <= go ? 2'd1 : 2'd0;
            2'd1: fsm_state <= go ? 2'd2 : 2'd0;
            2'd2: fsm_state <= 2'd3; // enters trap
            2'd3: fsm_state <= 2'd3; // true deadlock: no input can escape!
        endcase
    end
endmodule
