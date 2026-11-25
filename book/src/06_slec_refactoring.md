# Chapter 6: Sequential Equivalence Checking (SLEC) & Fearless Refactoring

**Sequential Logic Equivalence Checking (SLEC)** is the single highest-leverage safety net for both human RTL architects and AI coding agents: it mathematically proves that a refactored, pipelined, retimed, clock-gated, or safety-hardened `gate` design produces bit-for-bit identical outputs to a reference `gold` design on **every input sequence forever**.

## 6.1 Combinational & Same-Cycle Equivalence

```bash
veloformal equiv --gold tests/verilog/adder_gold.sv --gate tests/verilog/adder_gate.sv
```

VeloFormal builds a miter tying shared inputs together and asserting `gold.out == gate.out` for every output port, solved via k-induction + ABC PDR.

## 6.2 Pipelining & Retiming (`--latency-gold` / `--latency-gate`)

Suppose you convert a single-cycle combinational MAC (`mac_gold.sv`) into a 3-stage pipelined MAC (`mac_pipe3.sv`) to hit 1 GHz timing. Outputs now arrive 3 clock cycles later:

```bash
veloformal equiv \
    --gold examples/09_slec_pipelined_mac/mac_gold.sv \
    --gate examples/09_slec_pipelined_mac/mac_pipe3.sv \
    --latency-gold 3
```

VeloFormal inserts 3 delay registers on `gold`'s outputs, masks the initial 3 pipeline-fill cycles, and proves exact cycle-by-cycle equivalence for all $t \ge 3$.

## 6.3 Reset-Sequence Initialization (`--reset` & `--reset-cycles`)

When comparing designs whose internal registers initialize differently before hardware reset (or lack `initial` blocks), apply an explicit SLEC reset sequence:

```bash
veloformal equiv --gold orig.sv --gate retimed.sv --reset rst=1 --reset-cycles 2
```

For active-low reset, pass `--reset rst_n=0`.

## 6.4 Low-Power Clock Gating (`--valid-cond`) & Safety Insertion (`--ignore-output`)

When adding **Low-Power Clock Gating (LPGC)**, gated datapath registers intentionally hold stale values during idle cycles (`!out_valid`) to save dynamic switching power. Furthermore, adding safety logic (TMR/ECC) or power debug telemetry often introduces extra output ports (`err_flag`, `cg_active`) not present in `gold`.

VeloFormal handles both natively:

```bash
veloformal equiv \
    --gold examples/21_questa_parity/lpgc_gold.sv \
    --gate examples/21_questa_parity/lpgc_gate.sv \
    --reset rst=1 \
    --valid-cond out_valid \
    --ignore-output cg_active
```

If you omit `--valid-cond out_valid`, VeloFormal immediately returns `NOT EQUIVALENT` with the exact idle cycle where the gated register retains its previous value — proving the qualifier is doing real work.
