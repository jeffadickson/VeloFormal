// Tier 3 — Traditional IEEE 1800 SVA `bind` Spec (~115 lines — AND STILL INCOMPLETE!)
//
// Pain points:
//  1. 115+ lines of port re-declarations, `f_past_valid` reset guards, repeated `disable iff`,
//     manual handshake stability, cryptic SVA `|=>` / `##[1:3]` / `until`, and manual ghost FF logic.
//  2. EVEN AFTER 115 LINES OF SVA, IEEE 1800 SVA *CANNOT* EXPRESS:
//       - 4-state X-propagation freedom (`xfree resp_valid, busy, done, telemetry_out`)
//       - 2-copy Information Flow Confidentiality (`secure secret_key -> telemetry_out`)
//       - Conditional Declassification (`secure secret_key -> resp_data unless resp_is_auth`)
//     In traditional flows you must maintain SEPARATE vendor-specific Tcl scripts for X-Check and Secure!
module crypto_engine_sva (
    input logic       clk,
    input logic       rst,
    input logic       cmd_valid,
    input logic       cmd_ready,
    input logic [7:0] cmd_data,
    input logic       auth_ok,
    input logic [7:0] secret_key,
    input logic       resp_valid,
    input logic       resp_ready,
    input logic [7:0] resp_data,
    input logic       resp_is_auth,
    input logic       busy,
    input logic       done,
    input logic [3:0] telemetry_out
);
    // Boilerplate #1: Manual cycle-0 guard for $past() / $stable()
    logic f_past_valid;
    initial f_past_valid = 1'b0;
    always_ff @(posedge clk) begin
        if (rst) f_past_valid <= 1'b0;
        else     f_past_valid <= 1'b1;
    end

    // Boilerplate #2: Manual ghost counter register for job-accounting invariant
    logic [3:0] f_jobs_started;
    initial f_jobs_started = 4'd0;
    always_ff @(posedge clk) begin
        if (rst) begin
            f_jobs_started <= 4'd0;
        end else if (cmd_valid && cmd_ready && (f_jobs_started < 4'd15)) begin
            f_jobs_started <= f_jobs_started + 4'd1;
        end
    end

    // Boilerplate #3: Hand-coded response handshake stability under backpressure
    a_resp_valid_stable: assert property (
        @(posedge clk) disable iff (rst)
        (f_past_valid && $past(resp_valid) && !$past(resp_ready)) |-> resp_valid
    );

    a_resp_data_stable: assert property (
        @(posedge clk) disable iff (rst)
        (f_past_valid && $past(resp_valid) && !$past(resp_ready)) |-> $stable(resp_data)
    );

    // Boilerplate #4: Pipeline safety & ghost accounting invariants
    a_no_cmd_while_busy: assert property (
        @(posedge clk) disable iff (rst)
        busy |-> !cmd_ready
    );

    a_done_implies_busy: assert property (
        @(posedge clk) disable iff (rst)
        done |-> busy
    );

    a_completed_le_started: assert property (
        @(posedge clk) disable iff (rst)
        telemetry_out <= f_jobs_started
    );

    // Boilerplate #5: Multi-cycle temporal properties (cryptic SVA sequence syntax)
    a_launch_enters_busy: assert property (
        @(posedge clk) disable iff (rst)
        (cmd_valid && cmd_ready) |=> busy
    );

    a_launch_completes_within_3: assert property (
        @(posedge clk) disable iff (rst)
        (cmd_valid && cmd_ready) |-> ##[1:3] done
    );

    a_busy_until_done: assert property (
        @(posedge clk) disable iff (rst)
        (cmd_valid && cmd_ready) |=> (busy until done)
    );

    // Boilerplate #6: Reachability covers
    c_auth_job_completes: cover property (
        @(posedge clk) disable iff (rst)
        resp_valid && resp_is_auth
    );

    // MISSING (Impossible to express in SVA!):
    //   - 4-state X-freedom on resp_valid, busy, done, telemetry_out
    //   - 2-copy non-interference: secret_key never leaks to telemetry_out
    //   - 2-copy conditional declassification: secret_key never leaks to resp_data unless resp_is_auth
endmodule

bind crypto_engine crypto_engine_sva u_crypto_engine_sva (
    .clk(clk), .rst(rst),
    .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_data(cmd_data),
    .auth_ok(auth_ok), .secret_key(secret_key),
    .resp_valid(resp_valid), .resp_ready(resp_ready), .resp_data(resp_data),
    .resp_is_auth(resp_is_auth),
    .busy(busy), .done(done), .telemetry_out(telemetry_out)
);
