// Questa AutoCheck Deep Showcase:
// - FSM livelock: states 2 <-> 3 oscillate forever with no path back to IDLE (0) without reset!
// - FSM unreachable state: declared state 4 is never entered!
// - Shift-overflow bug: dynamic shift amount `shamt` (4-bit) can be >= 8 on an 8-bit value!
// - Clock-used-as-data structural lint: `clk_leak` uses `clk` in combinational data logic!
// - Constant-stuck register: `stuck_reg` never leaves its reset value 4'hA!
module fsm_livelock (
    input  logic       clk,
    input  logic       rst,
    input  logic       go,
    input  logic       bad_path,
    input  logic [7:0] din,
    input  logic [3:0] shamt,
    output logic [2:0] state,
    output logic [7:0] shifted,
    output logic       clk_leak,
    output logic [3:0] stuck_reg
);
    assign shifted  = din << shamt;  // BUG: shamt can be 8..15 >= 8!
    assign clk_leak = clk & go;      // WARN: clock used as data!

    initial begin
        state     = 3'd0;
        stuck_reg = 4'hA;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            state     <= 3'd0;
            stuck_reg <= 4'hA;
        end else begin
            stuck_reg <= 4'hA; // WARN: stuck constant forever!
            case (state)
                3'd0: state <= bad_path ? 3'd2 : (go ? 3'd1 : 3'd0);
                3'd1: state <= 3'd0;
                3'd2: state <= 3'd3; // LIVELOCK: 2 <-> 3 oscillates forever!
                3'd3: state <= 3'd2;
                3'd4: state <= 3'd0; // UNREACHABLE declared FSM state!
                default: state <= 3'd0;
            endcase
        end
    end
endmodule
