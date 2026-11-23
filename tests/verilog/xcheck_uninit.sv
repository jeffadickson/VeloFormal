// X-Check fixture: uninitialized flip-flop leaks X to `out_leak`, while `out_masked`
// masks X with a controlling 0, and `out_init` is properly initialized.
module xcheck_uninit (
    input  logic       clk,
    input  logic       rst,
    input  logic [7:0] din,
    output logic [7:0] out_leak,
    output logic [7:0] out_masked,
    output logic [7:0] out_init
);
    logic [7:0] uninit_reg; // No initial value -> starts X!
    logic [7:0] init_reg;
    initial init_reg = 8'h00;

    always_ff @(posedge clk) begin
        if (rst) begin
            init_reg <= 8'h00;
        end else begin
            uninit_reg <= din;
            init_reg   <= din;
        end
    end

    // Leaks X on cycle 0 (and whenever rst stays 1)!
    assign out_leak = uninit_reg ^ 8'hFF;

    // Controlling 0 masks X (0 & X == 0 in IEEE 1800)!
    assign out_masked = (uninit_reg & 8'h00) | init_reg;

    // Always X-free because init_reg has an explicit initial value!
    assign out_init = init_reg;
endmodule
