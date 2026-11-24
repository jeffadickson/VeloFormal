# Synopsys Design Constraints (SDC) for CDC analysis
create_clock -name wr_clk -period 10.0 [get_ports wr_clk]
create_clock -name rd_clk -period 12.5 [get_ports rd_clk]
set_clock_groups -asynchronous -group {wr_clk} -group {rd_clk}
set_false_path -from [get_clocks wr_clk] -to [get_clocks rd_clk]
