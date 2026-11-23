# VeloFormal

Native Verilog & SystemVerilog formal verification suite written in Rust, with an embedded [Slang](https://github.com/MikePopoloski/slang) IEEE 1800 frontend, word-level RTL netlist IR, SMT-LIB2 / BTOR2 / AIGER backends, full SymbiYosys-class engines, and Questa-class Formal Apps.

> The previous C#/Python prototype is preserved on the `archive/csharp-prototype` branch and `archive-csharp-prototype` tag.

## Architecture

```text
Verilog / SystemVerilog (.v / .sv / .sby)
  │
  ▼  vf-slang   — Embedded Slang v9.1 C++20 frontend + AST-to-IR lowering & RTL instrumentation
  ▼  vf-ir      — Word-level RTL netlist IR + passes (opt, coi_prune, chformal, async2sync, clk2fflogic)
  ▼  vf-smt     — SMT-LIB2 (QF_BV/QF_ABV), BTOR2, binary AIGER 1.0 bit-blaster, solver pipe, VCD writer
  ▼  vf-engine  — BMC, k-Induction, IC3/PDR (ABC), Liveness/Fairness (lasso), Cover, Vacuity, SLEC
  ▼  vf-apps    — Formal Apps: AutoCheck, CoverCheck, Connectivity, Secure (IFT), X-Check, Register Check, CDC/RDC
  ▼  vf-cli     — `veloformal` CLI & multi-task SymbiYosys (.sby) portfolio runner
  └── vip/      — Reusable Formal Assertion Library (Ready/Valid, AXI4-Lite, APB, FIFO scoreboard)
```

## Prerequisites

- Rust toolchain (`cargo`, `rustc`)
- C++20 compiler (`g++` 11+ / `clang++` 14+ / MSVC 2022) + `cmake` 3.20+ (plus `ninja` recommended)
- An SMT-LIB2 solver on `PATH`: `z3` (default), `bitwuzla`, or `cvc5`
- Optional: `yosys-abc` (or `abc`) for IC3/PDR proofs (`--engine pdr` / `abc pdr` in `.sby`)

## Build & Test

```bash
git submodule update --init --recursive
cargo build --release
cargo test --workspace
```

## Core Formal Verification (SymbiYosys / PropCheck / SLEC)

```bash
# Bounded Model Checking (BMC) with skip, append, and VCD counterexample
veloformal bmc tests/verilog/counter_fail.sv --depth 10 --append 2 --vcd counter_fail.vcd

# Unbounded safety proofs: k-induction, IC3/PDR, or auto, with optional COI pruning
veloformal prove tests/verilog/sync_fifo.sv --engine kind
veloformal prove tests/verilog/pdr_non_inductive.sv --engine pdr --coi
veloformal prove tests/verilog/pdr_non_inductive.sv --engine auto

# Assertion vacuity analysis (flags assertions whose antecedents are unreachable)
veloformal vacuity tests/verilog/vacuity_test.sv

# Liveness & fairness checking (bounded fair lasso detection)
veloformal live tests/verilog/liveness_starve.sv --depth 10 --vcd lasso.vcd

# Cover reachability witnesses (writes cover_0.vcd, ...)
veloformal cover tests/verilog/sync_fifo.sv --depth 10 --append 1 --vcd-dir waves/

# Full SymbiYosys (.sby) multi-task runner ([tasks], [file ...], portfolio engines, workdir status/VCD)
veloformal sby tests/verilog/multi_task.sby -d workdir/

# Sequential Logic Equivalence Checking (SLEC) with pipeline latency alignment & PDR
veloformal equiv --gold tests/verilog/adder_gold.sv --gate tests/verilog/adder_gate.sv
veloformal equiv --gold tests/verilog/slec_pipe_gold.sv --gate tests/verilog/slec_pipe_gate.sv --latency-gold 2

# Export IR, BTOR2, SMT-LIB2, or binary AIGER
veloformal dump-ir tests/verilog/sync_fifo.sv
veloformal dump-btor2 tests/verilog/sync_fifo.sv
veloformal dump-smt2 tests/verilog/sync_fifo.sv
veloformal dump-aiger tests/verilog/pdr_non_inductive.sv -o pdr.aig
```

## Formal Apps (Questa Formal Suite Equivalents)

```bash
# AutoCheck: push-button RTL bug hunting (OOB memory, div-by-zero, unique/priority, overflow, FSM deadlock)
veloformal autocheck tests/verilog/autocheck_bugs.sv --fsm fsm_state --vcd-dir waves/

# CoverCheck: automatic branch reachability witnesses & unbounded dead-code proofs
veloformal covercheck tests/verilog/covercheck_dead.sv

# Connectivity Check: static/conditional/inverted/delayed pin-to-pin routing rules from CSV
veloformal connectivity tests/verilog/mux_crossbar.sv --spec rules.csv

# Secure Check: formal Information Flow Tracking (2-copy self-composition + optional declassification)
veloformal secure tests/verilog/secure_crypto.sv --source secret_key --target pub_out --declassify debug_auth

# X-Check: 4-state dual-rail formal X-propagation verification (controlling values & uninitialized FFs)
veloformal xcheck tests/verilog/xcheck_uninit.sv --target out_leak --target out_masked

# Register Check: verify CSR reset values, RW/RO/W1C/W1S/WO policies, reserved bitmasks & non-aliasing
veloformal regcheck tests/verilog/csr_bank.sv --spec csr_map.csv

# CDC / RDC: structural clock/reset-domain crossing analyzer + formal Gray-code verification
veloformal cdc tests/verilog/cdc_design.sv --gray gray_cnt
```

## Formal Assertion Library (`vip/`)

Reusable SystemVerilog formal protocol checkers included in `vip/`:

- [`vip/vf_handshake.sv`](vip/vf_handshake.sv) — Ready/Valid stability under backpressure + transfer covers
- [`vip/vf_axi4_lite.sv`](vip/vf_axi4_lite.sv) — AMBA AXI4-Lite AW/W/B/AR/R channel compliance
- [`vip/vf_apb.sv`](vip/vf_apb.sv) — AMBA APB SETUP/ACCESS phase & wait-state stability
- [`vip/vf_fifo_props.sv`](vip/vf_fifo_props.sv) — Reusable FIFO safety & `$anyconst` data-integrity scoreboard

```bash
veloformal prove vip/vf_handshake.sv vip/vf_apb.sv tests/verilog/vip_dut_test.sv --top vip_dut_test
```

## SymbiYosys & SystemVerilog Formal Coverage

- **`.sby` Runner**: `[tasks]` with tag filtering (`tag:`, `~tag:`), inline `[file <name>]` blocks, `[options]` (`mode bmc|prove|cover|live`, `depth`, `skip`, `append`, `expect`, `multiclock`), `[engines]` (`smtbmc z3|bitwuzla|cvc5`, `abc pdr` with parallel portfolio racing), `[script]` (`read`/`read_verilog`, `prep`, `chparam`, `chformal`, `async2sync`, `clk2fflogic`), and SBY-compatible `<workdir>/status` + `engine_0/*.vcd` outputs.
- **RTL & SVA**: Full IEEE 1800 preprocessing (`` `ifdef FORMAL ``), hierarchy flattening, `generate` loops, continuous `assign`, `always_comb`/`always_ff`/`initial`, `if`/`else`, `case`/`casez`/`casex` wildcards, `unique`/`priority` qualifiers, procedural `for`/`repeat` loop unrolling, packed `struct` fields, pure SV `function` inlining, unpacked 1D memories (SMT `QF_ABV` arrays + bit-blasted AIGER), async resets (`async2sync`), multi-clock (`clk2fflogic`), immediate & concurrent SVA (`|->`, `|=>`, `##N`, `##[M:N]`, `disable iff`, `s_eventually`), labeled assertions, Yosys attributes (`(* anyseq *)`, `(* anyconst *)`, `(* gclk *)`), and system functions (`$past`, `$rose`, `$fell`, `$stable`, `$changed`, `$initstate`, `$anyseq`, `$anyconst`, `$onehot`, `$onehot0`, `$countones`).
