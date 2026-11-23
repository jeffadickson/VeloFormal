// Secure Check fixture: secret key only visible on pub_out when debug_auth == 1.
module secure_crypto (
    input  logic       clk,
    input  logic [7:0] pub_in,
    input  logic [7:0] secret_key,
    input  logic       debug_auth,
    output logic [7:0] pub_out,
    output logic [7:0] isolated_out
);
    logic [7:0] key_reg;
    initial key_reg = 8'h00;

    always_ff @(posedge clk) begin
        key_reg <= secret_key;
    end

    // Leaks `key_reg` ONLY when `debug_auth` is asserted (declassification).
    assign pub_out = debug_auth ? (pub_in ^ key_reg) : (pub_in + 8'd1);

    // Strictly isolated from `secret_key` for all time!
    assign isolated_out = pub_in ^ 8'h5A;
endmodule
