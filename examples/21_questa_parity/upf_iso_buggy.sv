// Questa Low-Power / IEEE 1801 UPF Power-Domain Isolation Example (Buggy)
// Bug: `pd_data` is NOT clamped to 0 when `iso_en == 1`!
module upf_iso_dut (
    input  logic       clk,
    input  logic       rst,
    input  logic       sleep_req,
    input  logic [7:0] raw_data,
    input  logic       raw_nready,
    input  logic [3:0] raw_status,
    output logic       iso_en,
    output logic       pd_off,
    output logic [7:0] pd_data,
    output logic       pd_nready,
    output logic [3:0] pd_status
);
    logic iso_r, off_r;
    initial begin
        iso_r = 1'b0;
        off_r = 1'b0;
    end
    always_ff @(posedge clk) begin
        if (rst) begin
            iso_r <= 1'b0;
            off_r <= 1'b0;
        end else if (sleep_req) begin
            iso_r <= 1'b1;
            off_r <= iso_r;
        end else begin
            off_r <= 1'b0;
            iso_r <= off_r;
        end
    end

    assign iso_en    = iso_r;
    assign pd_off    = off_r;
    assign pd_data   = raw_data; // BUG: missing isolation clamp to 0!
    assign pd_nready = iso_en ? 1'b1 : raw_nready;

    logic [3:0] status_hold;
    initial status_hold = 4'd0;
    always_ff @(posedge clk) begin
        if (rst)          status_hold <= 4'd0;
        else if (!iso_en) status_hold <= raw_status;
    end
    assign pd_status = iso_en ? status_hold : raw_status;
endmodule
