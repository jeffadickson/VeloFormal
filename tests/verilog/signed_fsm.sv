module signed_fsm (
    input  logic              clk,
    input  logic              rst_n,
    input  logic              step,
    input  logic signed [7:0] delta,
    output logic [2:0]        state,
    output logic signed [7:0] accum
);
    localparam logic [2:0] IDLE = 3'b001;
    localparam logic [2:0] RUN  = 3'b010;
    localparam logic [2:0] DONE = 3'b100;

    initial begin
        state = IDLE;
        accum = 8'sd0;
    end

    always_ff @(posedge clk) begin
        if (!rst_n) begin
            state <= IDLE;
            accum <= 8'sd0;
        end else begin
            case (state)
                IDLE: begin
                    if (step)
                        state <= RUN;
                end
                RUN: begin
                    if (accum >= 8'sd10)
                        state <= DONE;
                    else if (delta > 8'sd0 && delta <= 8'sd4)
                        accum <= accum + delta;
                end
                DONE: begin
                    if (!step)
                        state <= IDLE;
                end
                default: state <= IDLE;
            endcase
        end
    end

`ifdef FORMAL
    always_comb begin
        // One-hot FSM encoding invariant
        assert ($onehot(state));
        // Signed accumulator stays bounded
        assert (accum >= 8'sd0 && accum <= 8'sd13);
        cover (state == DONE);
    end
`endif
endmodule
