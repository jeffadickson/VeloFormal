// Example 01: Traffic Light Controller FSM
// Proves mutual exclusion (NS and EW never simultaneously green/yellow), safe green->yellow->red
// transitions, and covers a full intersection cycle.
module traffic_light (
    input  logic clk,
    input  logic rst,
    input  logic ew_car,
    output logic ns_green,
    output logic ns_yellow,
    output logic ns_red,
    output logic ew_green,
    output logic ew_yellow,
    output logic ew_red
);
    localparam logic [1:0] S_NS_GREEN  = 2'd0;
    localparam logic [1:0] S_NS_YELLOW = 2'd1;
    localparam logic [1:0] S_EW_GREEN  = 2'd2;
    localparam logic [1:0] S_EW_YELLOW = 2'd3;

    logic [1:0] state;
    logic [1:0] prev_state;
    logic       past_valid;

    initial begin
        state      = S_NS_GREEN;
        prev_state = S_NS_GREEN;
        past_valid = 1'b0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            state      <= S_NS_GREEN;
            prev_state <= S_NS_GREEN;
            past_valid <= 1'b0;
        end else begin
            past_valid <= 1'b1;
            prev_state <= state;
            case (state)
                S_NS_GREEN:  state <= ew_car ? S_NS_YELLOW : S_NS_GREEN;
                S_NS_YELLOW: state <= S_EW_GREEN;
                S_EW_GREEN:  state <= S_EW_YELLOW;
                S_EW_YELLOW: state <= S_NS_GREEN;
            endcase
        end
    end

    assign ns_green  = (state == S_NS_GREEN);
    assign ns_yellow = (state == S_NS_YELLOW);
    assign ns_red    = !(ns_green || ns_yellow);

    assign ew_green  = (state == S_EW_GREEN);
    assign ew_yellow = (state == S_EW_YELLOW);
    assign ew_red    = !(ew_green || ew_yellow);

`ifdef FORMAL
    always_comb begin
        // Mutual exclusion: never green/yellow on both roads simultaneously!
        a_mutex: assert (!((ns_green || ns_yellow) && (ew_green || ew_yellow)));

        // Exactly one light active on each road
        a_ns_onehot: assert ($onehot({ns_green, ns_yellow, ns_red}));
        a_ew_onehot: assert ($onehot({ew_green, ew_yellow, ew_red}));

        // Never jump directly from NS_GREEN to EW_GREEN without passing through NS_YELLOW
        if (past_valid && !rst) begin
            a_no_skip_yellow: assert (!(prev_state == S_NS_GREEN && state == S_EW_GREEN));
            a_no_skip_ew_yel: assert (!(prev_state == S_EW_GREEN && state == S_NS_GREEN));
        end

        // Cover reaching EW_GREEN and returning back to NS_GREEN after EW_YELLOW
        c_ew_green:   cover (!rst && ew_green);
        c_full_cycle: cover (past_valid && !rst && prev_state == S_EW_YELLOW && state == S_NS_GREEN);
    end
`endif
endmodule
