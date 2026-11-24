// Questa Low-Power / IEEE 1801 UPF Power-Domain Isolation Example (Clean)
// Properly clamps `pd_data` to 0, `pd_nready` to 1, and holds `pd_status` (latch) when `iso_en` is high.
// Also sequences `iso_en` active whenever `pd_off` is asserted.
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
    // Power controller: assert iso_en at cycle N, then pd_off at cycle N+1 (and keep iso_en while pd_off!).
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
            iso_r <= off_r; // Keep iso_en high until after power is restored!
        end
    end

    assign iso_en = iso_r;
    assign pd_off = off_r;

    // Isolation clamp cells:
    assign pd_data   = iso_en ? 8'h00 : raw_data;
    assign pd_nready = iso_en ? 1'b1  : raw_nready;

    // Latch-clamp cell: holds last value when `iso_en` is asserted.
    logic [3:0] status_hold;
    initial status_hold = 4'd0;
    always_ff @(posedge clk) begin
        if (rst)          status_hold <= 4'd0;
        else if (!iso_en) status_hold <= raw_status;
    end
    assign pd_status = iso_en ? status_hold : raw_status;
endmodule
