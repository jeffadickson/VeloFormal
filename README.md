# VeloFormal

Native Verilog & SystemVerilog formal verification tool written in Rust, with an embedded [Slang](https://github.com/MikePopoloski/slang) IEEE 1800 frontend, word-level RTL netlist IR, standard SMT-LIB2 / BTOR2 backends, and Yosys/SymbiYosys-class verification engines.

> The previous C#/Python prototype is preserved on the `archive/csharp-prototype` branch and `archive-csharp-prototype` tag.

## Architecture

```text
Verilog / SystemVerilog (.v / .sv with `ifdef FORMAL, assert/assume/cover, SVA, $past, $initstate, $anyconst)
  │
  ▼  vf-slang   — Embedded Slang v9.1 C++20 frontend + AST-to-IR lowering (hierarchy, generate, proc, memories)
  ▼  vf-ir      — Word-level RTL netlist IR (wires, cells, FFs, memories, properties) + opt passes
  ▼  vf-smt     — SMT-LIB2 (QF_BV / QF_ABV), BTOR2 exporter, incremental solver pipe (Z3 / Bitwuzla / CVC5), VCD writer
  ▼  vf-engine  — BMC, k-Induction unbounded prover, Cover witness finder, Equivalence checking (miter)
  ▼  vf-cli     — `veloformal` CLI binary
```

## Prerequisites

- Rust toolchain (`cargo`, `rustc`)
- C++20 compiler (`g++` 11+ / `clang++` 14+ / MSVC 2022) + `cmake` 3.20+ (plus `ninja` recommended)
- An SMT-LIB2 solver on `PATH`: `z3` (default), `bitwuzla`, or `cvc5`

## Build & Test

```bash
git submodule update --init --recursive
cargo build --release
cargo test
```

## Usage

```bash
# Bounded Model Checking (BMC) with VCD counterexample output
veloformal bmc tests/verilog/counter_fail.sv --depth 10 --vcd counter_fail.vcd

# Unbounded safety proof via k-induction
veloformal prove tests/verilog/counter_prove.sv --depth 20
veloformal prove tests/verilog/sync_fifo.sv --depth 20

# Cover reachability witnesses (writes cover_0.vcd, ...)
veloformal cover tests/verilog/sync_fifo.sv --depth 10 --vcd-dir waves/

# Execute a SymbiYosys (.sby) job directly
veloformal sby tests/verilog/sync_fifo.sby

# Combinational or sequential equivalence checking
veloformal equiv --gold tests/verilog/adder_gold.sv --gate tests/verilog/adder_gate.sv

# Inspect lowered RTL IR, BTOR2, or SMT-LIB2
veloformal dump-ir tests/verilog/sync_fifo.sv
veloformal dump-btor2 tests/verilog/sync_fifo.sv
veloformal dump-smt2 tests/verilog/sync_fifo.sv
```

## Supported SystemVerilog & Formal Constructs

- Full IEEE 1800 preprocessing (`` `define ``, `` `ifdef FORMAL ``, `` `include ``), parameters (`-G`), and `generate` loops/conditionals via embedded Slang
- Hierarchical module instantiation & flattening
- Continuous `assign`, `always_comb`, `always_ff`, `always @(*)`, `always @(posedge clk)`, `initial`
- `if`/`else`, `case`, blocking & nonblocking assignments, bit/range selects, concatenations, replications, signed/unsigned arithmetic & shifts
- 1D unpacked memories (arrays) lowered to SMT `QF_ABV` arrays & BTOR2 array states
- Immediate `assert`, `assume`, `cover`, `restrict` and concurrent SVA `assert/assume/cover property (@(posedge clk) ... |-> / |=> ...)`
- Formal system functions: `$past`, `$rose`, `$fell`, `$stable`, `$changed`, `$initstate`, `$anyseq`, `$anyconst`, `$onehot`, `$onehot0`
