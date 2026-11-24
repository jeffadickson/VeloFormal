# Questa-Formal-compatible .do / Tcl verification script
formal compile -d uart_csr ../15_regcheck_uart_csr/uart_csr.sv
netlist clock clk -period 10
netlist reset rst_n -active_low
regcheck verify -spec uart_csr.rdl
