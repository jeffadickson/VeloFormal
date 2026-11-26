# Chapter 5: Push-Button Formal Apps (Questa-Class Suite)

Formal Apps deliver massive bug-hunting and sign-off value with **zero SVA** (or lightweight declarative specs like CSV, SystemRDL, IP-XACT, and SDC).

## 5.1 AutoCheck — Push-Button RTL Bug Hunting (18 Checks)

AutoCheck instruments elaborated RTL automatically and proves or disproves common hardware defects in parallel across CPU cores — with zero SVA or testbench:

- **Indexing & Memory Safety**: Out-of-bounds memory/unpacked-array indexing (`oob_memory`) and dynamic bit-select / indexed part-select (`vec[idx]`, `vec[idx +: W]`, `vec[idx -: W]`) out-of-bounds (`oob_bitselect`)
- **Arithmetic & Shift Safety**: Divide/modulo by zero (`div_by_zero`), out-of-range dynamic shift amounts `rhs >= width(lhs)` (`shift_overflow`), optional signed & unsigned add/sub/mul overflow/underflow (`--check-overflow`), and optional formal implicit width-truncation / silent data-loss detection (`--check-truncation`)
- **Control Flow & Latch Safety**: `unique` / `unique0` / `priority` `case` & `if` overlap/completeness (`unique_priority`), plus **formal reachability-backed inferred latch detection** (`inferred_latch` — proves missing `else` on unreachable states `SAFE` while returning CEX traces for reachable latches, and allows intentional `always_latch`)
- **Structural & Clocking Hazards**: Combinational feedback loops (`comb_loop`, reported with full cycle path without aborting remaining checks), multi-driver bus contention (`multi_driven`, disjoint slice-aware), undriven wires (`undriven`), clock-used-as-data (`clock_as_data`), and optional unread/dead driven signals (`--check-unread`)
- **Register & FSM Health**: Automatic FSM state discovery (`--auto-fsm`) or explicit `--fsm <state_reg>` for deadlock (`fsm_deadlock`), multi-state livelock (`fsm_livelock`), and formally unreachable declared states (`fsm_unreachable`), plus optional constant-stuck registers (`--check-const-regs`) and formal unresettable register detection (`--check-no-reset`)

```bash
# Default push-button run:
veloformal autocheck examples/21_questa_parity/fsm_livelock.sv \
    --fsm state --check-const-regs --vcd-dir waves/

# Strict Superlint mode with rule waivers + JSON/SARIF/HTML reports:
veloformal autocheck rtl.sv --strict --disable unread_signal \
    --format json --sarif autocheck.sarif --report autocheck.html
```

## 5.2 CoverCheck — Reachable Witnesses & Dead-Code Proofs

CoverCheck instruments every `if`/`else`/`case` branch, generates shortest VCD witnesses for reachable branches, and **proves unbounded unreachability** for dead branches so you can exclude truly unreachable RTL from simulation coverage closure:

```bash
veloformal covercheck examples/11_covercheck_alu_ctrl/alu_controller.sv --vcd-dir cov_waves/
```

## 5.3 Connectivity Check — SoC Pinmux & Bus Routing

Prove static, conditional (`sel == 2`), inverted, or N-cycle pipelined pin-to-pin routing rules from a simple CSV (`src,dst[,cond[,invert[,delay]]]`):

```bash
veloformal connectivity examples/12_connectivity_soc_pinmux/soc_pinmux.sv \
    --spec examples/12_connectivity_soc_pinmux/pinmux_rules.csv
```

## 5.4 Secure Check — Formal Information Flow Tracking (IFT)

Constructs a 2-copy self-composition miter tying all public inputs equal while allowing secret/untrusted `--source` inputs to diverge, and proves that `--target` outputs remain strictly identical across both executions (outside an optional `--declassify` authorization guard):

```bash
veloformal secure examples/13_secure_key_vault/key_vault.sv \
    --source root_key --target telemetry_out

veloformal secure examples/13_secure_key_vault/key_vault.sv \
    --source root_key --target jtag_rdata --declassify "jtag_unlocked && auth_ok"
```

## 5.5 X-Check — 4-State Dual-Rail X-Propagation

Standard 2-state formal solvers silently mask uninitialized flip-flops. VeloFormal's X-Check encodes every signal as a dual-rail `(val, is_x)` pair, accurately propagating controlling values (`0 & X == 0`, `1 | X == 1`, mux selection):

```bash
veloformal xcheck examples/14_xcheck_dma_reset/dma_channel.sv \
    --target valid_out --target masked_data --target status_out
```

## 5.6 Register Check — CSV, SystemRDL 2.0, IP-XACT & AMBA APB

Synthesizes and proves CSR reset values, access policies (`RW`, `RO`, `W1C`, `W1S`, `WO`), reserved-bit masking, and cross-register non-aliasing directly from CSV, SystemRDL 2.0 (`.rdl`), or IEEE 1685 IP-XACT (`.xml`). Supports raw CSR buses, native **AMBA APB3/APB4** (`--protocol apb`), and 0- or 1-cycle registered read latency (`--read-latency 1`):

```bash
veloformal regcheck examples/15_regcheck_uart_csr/uart_csr.sv \
    --spec examples/19_industry_standards/uart_csr.rdl

veloformal regcheck examples/21_questa_parity/apb_csr.sv \
    --spec examples/21_questa_parity/apb_csr.rdl --protocol apb
```

## 5.7 CDC / RDC & SDC Constraints

Performs structural clock-domain crossing and reset-domain crossing analysis (2-FF synchronizer detection, reconvergence detection), honors Synopsys Design Constraints (`--sdc` with `create_clock`, `set_clock_groups -asynchronous`, `set_false_path`), and formally proves Gray-code single-bit transitions (`--gray`):

```bash
veloformal cdc examples/16_cdc_async_bridge/async_bridge.sv --gray wr_ptr_gray
veloformal cdc examples/16_cdc_async_bridge/async_bridge_buggy.sv \
    --sdc examples/19_industry_standards/async_bridge.sdc
```
