// Industry-Standard IEEE 1800 SVA: `bind` directive, named `property`, consecutive repetition `[*2]`,
// and `until` operator attached non-invasively to a pulse stretcher DUT!
module pulse_stretcher (
    input  logic clk,
    input  logic rst,
    input  logic trig,
    output logic busy,
    output logic done
);
    logic [1:0] cnt;
    initial cnt = 2'd0;

    assign busy = (cnt != 2'd0);
    assign done = (cnt == 2'd1);

    always_ff @(posedge clk) begin
        if (rst) begin
            cnt <= 2'd0;
        end else if (cnt == 2'd0) begin
            if (trig) cnt <= 2'd3;
        end else begin
            cnt <= cnt - 2'd1;
        end
    end
endmodule

module pulse_stretcher_sva (
    input logic clk,
    input logic rst,
    input logic trig,
    input logic busy,
    input logic done
);
    // Named SVA property using consecutive repetition `[*3]`
    property p_stretch_3;
        @(posedge clk) disable iff (rst)
        (!busy && trig) |=> busy[*3];
    endproperty

    // Named SVA property using `until` operator
    property p_busy_until_done;
        @(posedge clk) disable iff (rst)
        (!busy && trig) |=> (busy until done);
    endproperty

    a_stretch_3:       assert property (p_stretch_3);
    a_busy_until_done: assert property (p_busy_until_done);
endmodule

// Non-invasive IEEE 1800 `bind` attaches the SVA checker to pulse_stretcher!
bind pulse_stretcher pulse_stretcher_sva u_sva (
    .clk(clk),
    .rst(rst),
    .trig(trig),
    .busy(busy),
    .done(done)
);
