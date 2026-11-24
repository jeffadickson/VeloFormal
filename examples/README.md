# VeloFormal Examples

Nineteen self-contained, verified formal verification examples covering every SymbiYosys-class engine, Questa-class Formal App, `.vfc` v1.0 Contract DSL, industry-standard formats (SVA `bind`, SystemRDL, IP-XACT, SDC, Questa Tcl), and reusable `vip/` Formal Assertion Library checker.

All examples are executed continuously in CI via [`examples_suite.rs`](../crates/vf-cli/tests/examples_suite.rs) and [`dsl_and_standards.rs`](../crates/vf-cli/tests/dsl_and_standards.rs).

| # | Example | Feature / App | Expected Result |
|---|---------|---------------|-----------------|
| 01 | [`01_traffic_light_fsm`](01_traffic_light_fsm/) | FSM safety, cover & multi-task `.sby` | `PASS` / `PROVED` |
| 02 | [`02_skid_buffer_vip`](02_skid_buffer_vip/) | Elastic ready/valid skid buffer + `vip/vf_handshake.sv` + `$anyconst` scoreboard | `PROVED` + `COVER` |
| 03 | [`03_fifo_scoreboard_vip`](03_fifo_scoreboard_vip/) | Synchronous FIFO + `vip/vf_fifo_props.sv` data-integrity scoreboard | `PROVED` + `COVER` |
| 04 | [`04_axi4_lite_slave_vip`](04_axi4_lite_slave_vip/) | AMBA AXI4-Lite slave + 5-channel `vip/vf_axi4_lite.sv` monitor | `PROVED` |
| 05 | [`05_apb_bridge_vip`](05_apb_bridge_vip/) | AMBA APB master bridge + `vip/vf_apb.sv` monitor | `PROVED` |
| 06 | [`06_arbiter_liveness`](06_arbiter_liveness/) | 4-way Round-Robin safety + liveness vs Fixed-Priority starvation lasso | `PROVED`/`PASS` & `FAIL` lasso |
| 07 | [`07_uart_tx_sva`](07_uart_tx_sva/) | UART TX with multi-cycle concurrent SVA (`\|=>`, `\|-> ##10`, `disable iff`) | `PROVED` + `COVER` |
| 08 | [`08_pdr_credit_counter`](08_pdr_credit_counter/) | Non-inductive credit-flow invariant proved by IC3/PDR (`abc pdr`) | `PROVED` via PDR |
| 09 | [`09_slec_pipelined_mac`](09_slec_pipelined_mac/) | SLEC latency-aligned 3-stage pipelined MAC equivalence (`--latency-gold 3`) | `EQUIVALENT` & `NOT EQUIVALENT` |
| 10 | [`10_autocheck_packet_parser`](10_autocheck_packet_parser/) | AutoCheck zero-SVA bug hunting (OOB LUT, div-by-zero, `unique case`, FSM deadlock) | `BUG` (x4) & `SAFE` |
| 11 | [`11_covercheck_alu_ctrl`](11_covercheck_alu_ctrl/) | CoverCheck branch witnesses + unbounded dead-code unreachability proofs | `REACHABLE` + `UNREACHABLE` |
| 12 | [`12_connectivity_soc_pinmux`](12_connectivity_soc_pinmux/) | Connectivity Check CSV verification (mux, inverted IRQ, 1-cycle pipeline) | All 5 rules `PROVED` |
| 13 | [`13_secure_key_vault`](13_secure_key_vault/) | Secure Check Information Flow Tracking (isolation & `--declassify`) | `SECURE`, `LEAK`, `SECURE` |
| 14 | [`14_xcheck_dma_reset`](14_xcheck_dma_reset/) | X-Check 4-state X-propagation (missing reset vs controlling-0 masking) | `X-FREE` & `X-LEAK` |
| 15 | [`15_regcheck_uart_csr`](15_regcheck_uart_csr/) | Register Check CSR bank (RW, RO, W1C, W1S, WO, masks, non-aliasing) | All 16 checks `PASS` |
| 16 | [`16_cdc_async_bridge`](16_cdc_async_bridge/) | CDC / RDC structural analysis + formal Gray-code verification (`--gray`) | Clean `SAFE`/`GRAY PROVED` & buggy hazards |
| 17 | [`17_vacuity_detection`](17_vacuity_detection/) | PropCheck vacuity check catching assertions with unreachable guards | `NON-VACUOUS` & `VACUOUS` |
| 18 | [`18_vfc_contracts`](18_vfc_contracts/) | `.vfc` v1.0 Contract DSL (counter, FIFO scoreboard, key vault security + xfree, `emit-sva`) | `PROVED`, `REACHED`, `SECURE`, `X-FREE` |
| 19 | [`19_industry_standards`](19_industry_standards/) | IEEE 1800 SVA `bind`/`[*3]`/`until`, SystemRDL `.rdl`, IP-XACT `.xml`, SDC `.sdc`, Questa `.do` Tcl + SARIF/HTML | `PROVED` / `PASS` / `SAFE` |
| 20 | [`20_riscv_rv32i_formal`](20_riscv_rv32i_formal/) | Real 2-stage pipelined RV32I RISC-V Core verified via `.vfc` ISA contract + X-Check | `PROVED`, `X-FREE`, `REACHED` |

## Commands

Run from the repository root (replace `veloformal` with `cargo run --release -p vf-cli --` if not installed on `PATH`):

```bash
# 01: Traffic Light FSM (multi-task .sby: prove + bmc + cover)
veloformal sby examples/01_traffic_light_fsm/traffic_light.sby

# 02: Elastic Ready/Valid Skid Buffer + Formal VIP Handshake Checker
veloformal prove vip/vf_handshake.sv examples/02_skid_buffer_vip/skid_buffer.sv --top skid_buffer
veloformal cover vip/vf_handshake.sv examples/02_skid_buffer_vip/skid_buffer.sv --top skid_buffer

# 03: Synchronous FIFO + Reusable Formal VIP Scoreboard
veloformal prove vip/vf_fifo_props.sv examples/03_fifo_scoreboard_vip/sync_fifo_vip.sv --top sync_fifo_vip --engine auto

# 04: AMBA AXI4-Lite Slave + 5-Channel Formal VIP Monitor
veloformal prove vip/vf_handshake.sv vip/vf_axi4_lite.sv examples/04_axi4_lite_slave_vip/axi4_lite_slave.sv --top axi4_lite_slave

# 05: AMBA APB Bridge + Formal VIP APB Monitor
veloformal prove vip/vf_apb.sv examples/05_apb_bridge_vip/apb_bridge_vip.sv --top apb_bridge_vip

# 06: 4-Way Round-Robin Arbiter (Safety + Liveness) vs Fixed-Priority Starvation
veloformal prove examples/06_arbiter_liveness/rr_arbiter_4.sv
veloformal live  examples/06_arbiter_liveness/rr_arbiter_4.sv --depth 8
veloformal live  examples/06_arbiter_liveness/fixed_prio_starve.sv --depth 6 --vcd starve.vcd

# 07: UART Transmitter with Concurrent SVA Sequences
veloformal prove examples/07_uart_tx_sva/uart_tx.sv --engine auto
veloformal cover examples/07_uart_tx_sva/uart_tx.sv --depth 15

# 08: Non-Inductive Credit Flow Invariant (IC3/PDR)
veloformal sby examples/08_pdr_credit_counter/credit_flow.sby

# 09: SLEC 3-Stage Pipelined MAC Equivalence
veloformal equiv --gold examples/09_slec_pipelined_mac/mac_gold.sv --gate examples/09_slec_pipelined_mac/mac_pipe3.sv --latency-gold 3
veloformal equiv --gold examples/09_slec_pipelined_mac/mac_gold.sv --gate examples/09_slec_pipelined_mac/mac_pipe3_bug.sv --latency-gold 3 --vcd mac_bug.vcd

# 10: AutoCheck Push-Button RTL Bug Hunting (zero SVA!)
veloformal autocheck examples/10_autocheck_packet_parser/packet_parser_buggy.sv --fsm parse_state
veloformal autocheck examples/10_autocheck_packet_parser/packet_parser_fixed.sv --fsm parse_state

# 11: CoverCheck Reachable Branches & Unbounded Dead-Code Proofs
veloformal covercheck examples/11_covercheck_alu_ctrl/alu_controller.sv

# 12: Connectivity Check SoC Pinmux & Interrupt Router
veloformal connectivity examples/12_connectivity_soc_pinmux/soc_pinmux.sv --spec examples/12_connectivity_soc_pinmux/pinmux_rules.csv

# 13: Secure Check Hardware Key Vault (Information Flow Tracking)
veloformal secure examples/13_secure_key_vault/key_vault.sv --source root_key --target telemetry_out
veloformal secure examples/13_secure_key_vault/key_vault.sv --source root_key --target jtag_rdata
veloformal secure examples/13_secure_key_vault/key_vault.sv --source root_key --target jtag_rdata --declassify "jtag_unlocked && auth_ok"

# 14: X-Check 4-State Reset & X-Propagation Analysis
veloformal xcheck examples/14_xcheck_dma_reset/dma_channel.sv --target valid_out --target masked_data
veloformal xcheck examples/14_xcheck_dma_reset/dma_channel.sv --target status_out

# 15: Register Check Full UART CSR Bank
veloformal regcheck examples/15_regcheck_uart_csr/uart_csr.sv --spec examples/15_regcheck_uart_csr/uart_csr.csv

# 16: CDC / RDC Structural Analysis + Formal Gray-Code Proof
veloformal cdc examples/16_cdc_async_bridge/async_bridge.sv --gray wr_ptr_gray
veloformal cdc examples/16_cdc_async_bridge/async_bridge_buggy.sv

# 17: PropCheck Vacuity Detection
veloformal vacuity examples/17_vacuity_detection/arbiter_vacuous.sv

# 18: VeloFormal Contract DSL (.vfc v1.0) — Non-invasive RTL contracts + HTML dashboard + SVA export
veloformal contract examples/18_vfc_contracts/counter.sv --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --vacuity --report signoff.html
veloformal contract examples/18_vfc_contracts/fifo_rtl.sv --spec examples/18_vfc_contracts/fifo_contract.vfc
veloformal contract examples/13_secure_key_vault/key_vault.sv --spec examples/18_vfc_contracts/key_vault.vfc
veloformal emit-sva examples/18_vfc_contracts/counter.vfc --rtl examples/18_vfc_contracts/counter.sv -o counter_sva.sv
veloformal fmt examples/18_vfc_contracts/counter.vfc --check

# 19: Industry-Standard Questa Replacement (SVA bind/[*3]/until, SystemRDL, IP-XACT, SDC, Questa Tcl)
veloformal prove examples/19_industry_standards/sva_bind_and_rep.sv --top pulse_stretcher
veloformal regcheck examples/15_regcheck_uart_csr/uart_csr.sv --spec examples/19_industry_standards/uart_csr.rdl
veloformal regcheck examples/15_regcheck_uart_csr/uart_csr.sv --spec examples/19_industry_standards/uart_csr.xml
veloformal cdc examples/16_cdc_async_bridge/async_bridge_buggy.sv --sdc examples/19_industry_standards/async_bridge.sdc
veloformal tcl examples/19_industry_standards/questa_signoff.do --format sarif --report signoff.html

# 20: Real 2-Stage Pipelined RV32I RISC-V Core Formal ISA Verification + X-Check
veloformal contract examples/20_riscv_rv32i_formal/rv32i_core.sv --spec examples/20_riscv_rv32i_formal/rv32i_isa.vfc --report rv32i_signoff.html

# Scalability & Sign-Off Coverage (Portfolio, Case-Splitting, COI Cache, Proof Core, Mutation Testing)
veloformal prove tests/verilog/pdr_non_inductive.sv --engine portfolio --cache-dir .vf-cache
veloformal prove examples/01_traffic_light_fsm/traffic_light.sv --case-split ew_car=0..1
veloformal proof-core examples/06_arbiter_liveness/rr_arbiter_4.sv
veloformal mutate examples/18_vfc_contracts/counter.sv --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --report mutate.html
```
