# Chapter 2: Getting Started in 5 Minutes

## 2.1 Installation & Prerequisites

VeloFormal compiles to native Rust binaries (`veloformal`, `vf-lsp`, `vf-mcp`):

- **Rust** (stable toolchain)
- **C++20 compiler** (`g++` 11+ / `clang++` 14+) + `cmake` 3.20+ (builds embedded Slang v9.1 once and caches it)
- **SMT Solver** on `PATH`: `z3` (default), `bitwuzla`, or `cvc5`
- **Optional**: `yosys-abc` (or `abc`) for IC3/PDR unbounded proofs

```bash
git clone --recursive https://github.com/jeffadickson/VeloFormal.git
cd VeloFormal
cargo build --release --workspace
cargo test --workspace
```

Or build the batteries-included Docker image (ships `veloformal`, `vf-lsp`, `vf-mcp`, `z3`, and `yosys-abc`):

```bash
docker build -t veloformal .
docker run --rm -v "$PWD:/work" veloformal --help
```

## 2.2 Step 1 — Inspect Your RTL (Zero Guessing)

Start by inspecting any Verilog or SystemVerilog file. VeloFormal elaborates it through Slang and prints exact ports, widths, clocks, reset polarity, registers, FSM candidates, and detected bus protocols:

```bash
veloformal inspect examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.sv
```

For scripting or AI agents, append `--format json`.

## 2.3 Step 2 — Run Push-Button AutoCheck (Zero Spec Required)

Before writing a single property, run AutoCheck to hunt OOB memory accesses, divide-by-zero, shift-overflow, `unique`/`priority` case violations, FSM deadlocks/livelocks/unreachable states, and structural hazards:

```bash
veloformal autocheck examples/21_questa_parity/fsm_livelock.sv --fsm state --check-const-regs
```

## 2.4 Step 3 — Scaffold & Prove a `.vfc` Contract

Generate a starter contract automatically, refine it, and verify safety, reachability covers, vacuity, and generate an Interactive HTML Sign-Off Dashboard with embedded SVG waveforms:

```bash
veloformal scaffold examples/18_vfc_contracts/fifo_rtl.sv -o my_fifo.vfc

veloformal contract examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv \
    --spec examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.vfc \
    --vacuity --report crypto_signoff.html
```

## 2.5 Step 4 — Debug Counterexamples Without VCD Overload

Whenever a safety assertion fails, VeloFormal prints a compact Cone-of-Influence (COI) **Root-Cause Backtrace** right in your terminal showing only the relevant signals that toggled leading into the failure cycle:

```bash
veloformal bmc tests/verilog/counter_fail.sv --depth 10 --vcd counter_fail.vcd
```

## 2.6 Step 5 — Pass the 4-Gate Trust Audit

Before merging or claiming formal sign-off, run `veloformal audit`:

```bash
veloformal audit examples/18_vfc_contracts/counter.sv \
    --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5
```

`AUDIT TRUSTWORTHY` confirms: (1) unbounded proof passed, (2) zero assumption dead-ends, (3) zero vacuous assertions, and (4) strong formal mutation kill-rate.
