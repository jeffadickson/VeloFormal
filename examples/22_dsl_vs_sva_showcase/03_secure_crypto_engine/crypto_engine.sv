// Tier 3 RTL: Multi-Cycle Secure Crypto Accelerator Engine (Pure Production RTL — Zero `ifdef FORMAL)
// Features:
//  - Ready/Valid command & response streams with backpressure
//  - 2-cycle pipelined cryptographic round FSM (`IDLE -> ROUND1 -> ROUND2 -> IDLE`)
//  - Authorization-gated secret key (`secret_key` only enters datapath when `auth_ok == 1`)
//  - Public job-completion telemetry counter (`telemetry_out` — isolated from `secret_key`)
module crypto_engine (
    input  logic       clk,
    input  logic       rst,
    input  logic       cmd_valid,
    output logic       cmd_ready,
    input  logic [7:0] cmd_data,
    input  logic       auth_ok,
    input  logic [7:0] secret_key,
    output logic       resp_valid,
    input  logic       resp_ready,
    output logic [7:0] resp_data,
    output logic       resp_is_auth,
    output logic       busy,
    output logic       done,
    output logic [3:0] telemetry_out
);
    localparam logic [1:0] S_IDLE   = 2'd0;
    localparam logic [1:0] S_ROUND1 = 2'd1;
    localparam logic [1:0] S_ROUND2 = 2'd2;

    logic [1:0] state;
    logic [7:0] work_data;
    logic [7:0] work_key;
    logic       work_auth;

    assign busy      = (state != S_IDLE);
    assign done      = (state == S_ROUND2);
    assign cmd_ready = (state == S_IDLE) && (!resp_valid || resp_ready);

    initial begin
        state         = S_IDLE;
        work_data     = 8'd0;
        work_key      = 8'd0;
        work_auth     = 1'b0;
        resp_valid    = 1'b0;
        resp_data     = 8'd0;
        resp_is_auth  = 1'b0;
        telemetry_out = 4'd0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            state         <= S_IDLE;
            work_data     <= 8'd0;
            work_key      <= 8'd0;
            work_auth     <= 1'b0;
            resp_valid    <= 1'b0;
            resp_data     <= 8'd0;
            resp_is_auth  <= 1'b0;
            telemetry_out <= 4'd0;
        end else begin
            // Clear response buffer when consumer accepts it
            if (resp_valid && resp_ready) begin
                resp_valid   <= 1'b0;
                resp_is_auth <= 1'b0;
                resp_data    <= 8'd0;
            end

            case (state)
                S_IDLE: begin
                    if (cmd_valid && cmd_ready) begin
                        state     <= S_ROUND1;
                        work_data <= cmd_data;
                        work_auth <= auth_ok;
                        // Security isolation: never latch secret_key when unauthorized!
                        work_key  <= auth_ok ? secret_key : 8'h00;
                    end
                end
                S_ROUND1: begin
                    work_data <= (work_data ^ work_key) + 8'h13;
                    state     <= S_ROUND2;
                end
                S_ROUND2: begin
                    resp_valid   <= 1'b1;
                    resp_data    <= work_data ^ 8'hA5;
                    resp_is_auth <= work_auth;
                    if (telemetry_out < 4'd15) begin
                        telemetry_out <= telemetry_out + 4'd1;
                    end
                    state <= S_IDLE;
                end
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
