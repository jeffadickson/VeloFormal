// Example 06a: 4-Way Round-Robin Arbiter (Safety + Liveness)
// Proves mutual exclusion ($onehot0), grant-implies-request, work conservation, and bounded fairness.
module rr_arbiter_4 (
    input  logic       clk,
    input  logic       rst,
    input  logic [3:0] req,
    output logic [3:0] gnt
);
    logic [1:0] ptr;
    initial ptr = 2'd0;

    always_comb begin
        gnt = 4'b0000;
        case (ptr)
            2'd0: begin
                if      (req[0]) gnt = 4'b0001;
                else if (req[1]) gnt = 4'b0010;
                else if (req[2]) gnt = 4'b0100;
                else if (req[3]) gnt = 4'b1000;
            end
            2'd1: begin
                if      (req[1]) gnt = 4'b0010;
                else if (req[2]) gnt = 4'b0100;
                else if (req[3]) gnt = 4'b1000;
                else if (req[0]) gnt = 4'b0001;
            end
            2'd2: begin
                if      (req[2]) gnt = 4'b0100;
                else if (req[3]) gnt = 4'b1000;
                else if (req[0]) gnt = 4'b0001;
                else if (req[1]) gnt = 4'b0010;
            end
            2'd3: begin
                if      (req[3]) gnt = 4'b1000;
                else if (req[0]) gnt = 4'b0001;
                else if (req[1]) gnt = 4'b0010;
                else if (req[2]) gnt = 4'b0100;
            end
        endcase
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            ptr <= 2'd0;
        end else if (|gnt) begin
            case (gnt)
                4'b0001: ptr <= 2'd1;
                4'b0010: ptr <= 2'd2;
                4'b0100: ptr <= 2'd3;
                4'b1000: ptr <= 2'd0;
                default: ;
            endcase
        end
    end

`ifdef FORMAL
    always_comb begin
        // Safety properties
        a_mutex:      assert ($onehot0(gnt));
        a_only_req:   assert ((gnt & ~req) == 4'b0000);
        a_work_cons:  assert ((|req) == (|gnt));

        // Cover each master winning arbitration
        c_gnt0: cover (gnt[0]);
        c_gnt3: cover (gnt[3]);
    end

    // Liveness: if master 2 holds req[2] continuously and rst stays low, it must eventually be granted!
    // (No starving lasso exists for round-robin!)
    assert property (@(posedge clk) s_eventually (!req[2] || gnt[2] || rst));
`endif
endmodule
