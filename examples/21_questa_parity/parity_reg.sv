// ISO 26262 Functional Safety Example 2: Parity-Protected Register with Safety Alarm
// Any single-bit SA0, SA1, or SEU fault in `data_reg` CAN propagate to `dout`,
// but is 100% DETECTED by the even-parity safety mechanism (`parity_err == 1`)!
// Run: veloformal fault examples/21_questa_parity/parity_reg.sv --target data_reg --alarm parity_err
module parity_reg (
    input  logic       clk,
    input  logic       rst,
    input  logic [3:0] din,
    output logic [3:0] dout,
    output logic       parity_err
);
    logic [3:0] data_reg;
    logic       par_reg;

    initial begin
        data_reg = 4'd0;
        par_reg  = 1'b0;
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            data_reg <= 4'd0;
            par_reg  <= 1'b0;
        end else begin
            data_reg <= din;
            par_reg  <= ^din;
        end
    end

    assign dout       = data_reg;
    assign parity_err = (^data_reg) ^ par_reg;
endmodule
