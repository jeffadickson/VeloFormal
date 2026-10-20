`timescale 1ns/1ps
module tb;
  reg clk = 0;
  always #5 clk = !clk;
  reg resetn = 0;
  reg [3:0] reg_div_we = 0;
  reg [31:0] reg_div_di = 0;
  wire [31:0] actual;
  reg [31:0] expected = 1;
  simpleuart dut(.clk(clk), .resetn(resetn), .ser_tx(), .ser_rx(1'b1),
    .reg_div_we(reg_div_we), .reg_div_di(reg_div_di), .reg_div_do(actual),
    .reg_dat_we(1'b0), .reg_dat_re(1'b0), .reg_dat_di(32'b0),
    .reg_dat_do(), .reg_dat_wait());

  // Independent reference uses a word-wide mask rather than byte assignments.
  reg [31:0] mask;
  always @(posedge clk) begin
    mask = { {8{reg_div_we[3]}}, {8{reg_div_we[2]}},
             {8{reg_div_we[1]}}, {8{reg_div_we[0]}} };
    if (!resetn) expected <= 1;
    else expected <= (expected & ~mask) | (reg_div_di & mask);
  end
  integer pattern, byte_mask;
  initial begin
    $dumpfile("divider.vcd");
    $dumpvars(0, tb);
    repeat (2) @(negedge clk);
    resetn = 1;
    for (pattern = 0; pattern < 3; pattern = pattern + 1) begin
      for (byte_mask = 0; byte_mask < 16; byte_mask = byte_mask + 1) begin
        @(negedge clk);
        reg_div_we = byte_mask;
        case (pattern)
          0: reg_div_di = 32'h12345678;
          1: reg_div_di = 32'hfedcba98;
          2: reg_div_di = 32'h00000000;
        endcase
      end
    end
    @(negedge clk); reg_div_we = 0;
    repeat (2) @(negedge clk);
    resetn = 0;
    @(negedge clk); resetn = 1;
    repeat (2) @(negedge clk);
    $finish;
  end
endmodule
