// Example 13: Hardware Key Vault for Secure Check (Information Flow Tracking)
// - `telemetry_out` is strictly non-interfering with `root_key` (SECURE!)
// - `jtag_rdata` leaks `root_key` unless authorized by `jtag_unlocked && auth_ok`
//   (LEAK without `--declassify`, SECURE with `--declassify "jtag_unlocked && auth_ok"`!)
module key_vault (
    input  logic        clk,
    input  logic        rst,
    input  logic [31:0] root_key,
    input  logic [31:0] pub_nonce,
    input  logic        jtag_unlocked,
    input  logic        auth_ok,
    output logic [31:0] telemetry_out,
    output logic [31:0] jtag_rdata
);
    logic [31:0] key_shadow;
    logic [31:0] nonce_reg;

    initial begin
        key_shadow = 32'h0;
        nonce_reg  = 32'h0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            key_shadow <= 32'h0;
            nonce_reg  <= 32'h0;
        end else begin
            key_shadow <= root_key;
            nonce_reg  <= pub_nonce + 32'd1;
        end
    end

    // Strictly isolated public telemetry output
    assign telemetry_out = nonce_reg ^ 32'hA5A5_5A5A;

    // Debug readback only exposes key_shadow when both jtag_unlocked and auth_ok are high
    assign jtag_rdata = (jtag_unlocked && auth_ok) ? key_shadow : nonce_reg;
endmodule
