# VeloFormal

Native Verilog & SystemVerilog formal verification tool written in Rust, with an embedded [Slang](https://github.com/MikePopoloski/slang) IEEE 1800 frontend, word-level RTL netlist IR, SMT-LIB2 / BTOR2 / AIGER backends, and full SymbiYosys-class formal verification engines.

> The previous C#/Python prototype is preserved on the `archive/csharp-prototype` branch and `archive-csharp-prototype` tag.

## Architecture

```text
Verilog / SystemVerilog (.v / .sv / .sby)
  │
  ▼  vf-slang   — Embedded Slang v9.1 C++20 frontend + AST-to-IR lowering
  ▼  vf-ir      — Word-level RTL netlist IR + formal passes (opt, chformal, async2sync, clk2fflogic)
  ▼  vf-smt     — SMT-LIB2 (QF_BV/QF_ABV), BTOR2, binary AIGER 1.0 bit-blaster, solver pipe, VCD writer
  ▼  vf-engine  — BMC, k-Induction, IC3/PDR (ABC), Liveness/Fairness (lasso), Cover, Equivalence (miter)
  ▼  vf-cli     — `veloformal` CLI & multi-task SymbiYosys (.sby) portfolio runner
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

## Usage

```bash
# Bounded Model Checking (BMC) with skip, append, and VCD counterexample
veloformal bmc tests/verilog/counter_fail.sv --depth 10 --append 2 --vcd counter_fail.vcd

# Unbounded safety proofs: k-induction, IC3/PDR (proves non-inductive invariants!), or auto
veloformal prove tests/verilog/sync_fifo.sv --engine kind
veloformal prove tests/verilog/pdr_non_inductive.sv --engine pdr
veloformal prove tests/verilog/pdr_non_inductive.sv --engine auto

# Liveness & fairness checking (bounded fair lasso detection)
veloformal live tests/verilog/liveness_starve.sv --depth 10 --vcd lasso.vcd

# Cover reachability witnesses (writes cover_0.vcd, ...)
veloformal cover tests/verilog/sync_fifo.sv --depth 10 --append 1 --vcd-dir waves/

# Full SymbiYosys (.sby) multi-task runner ([tasks], [file ...], portfolio engines, workdir status/VCD)
veloformal sby tests/verilog/multi_task.sby -d workdir/

# Combinational or sequential equivalence checking
veloformal equiv --gold tests/verilog/adder_gold.sv --gate tests/verilog/adder_gate.sv
veloformal equiv --gold tests/verilog/shift_gold.sv --gate tests/verilog/shift_gate.sv

# Export IR, BTOR2, SMT-LIB2, or binary AIGER
veloformal dump-ir tests/verilog/sync_fifo.sv
veloformal dump-btor2 tests/verilog/sync_fifo.sv
veloformal dump-smt2 tests/verilog/sync_fifo.sv
veloformal dump-aiger tests/verilog/pdr_non_inductive.sv -o pdr.aig
```

## SymbiYosys & SystemVerilog Formal Coverage

- **`.sby` Runner**: `[tasks]` with tag filtering (`tag:`, `~tag:`), inline `[file <name>]` blocks, `[options]` (`mode bmc|prove|cover|live`, `depth`, `skip`, `append`, `expect`, `multiclock`), `[engines]` (`smtbmc z3|bitwuzla|cvc5`, `abc pdr` with parallel portfolio racing), `[script]` (`read`/`read_verilog`, `prep`, `chparam`, `chformal`, `async2sync`, `clk2fflogic`), and SBY-compatible `<workdir>/status` + `engine_0/*.vcd` outputs.
- **RTL & SVA**: Full IEEE 1800 preprocessing (`` `ifdef FORMAL ``), hierarchy flattening, `generate` loops, continuous `assign`, `always_comb`/`always_ff`/`initial`, `if`/`else`, `case`/`casez`/`casex` wildcards, procedural `for`/`repeat` loop unrolling, packed `struct` fields, pure SV `function` inlining, unpacked 1D memories (SMT `QF_ABV` arrays + bit-blasted AIGER), async resets (`async2sync`), multi-clock (`clk2fflogic`), immediate & concurrent SVA (`|->`, `|=>`, `##N`, `##[M:N]`, `disable iff`, `s_eventually`), Yosys attributes (`(* anyseq *)`, `(* anyconst *)`, `(* gclk *)`), and system functions (`$past`, `$rose`, `$fell`, `$stable`, `$changed`, `$initstate`, `$anyseq`, `$anyconst`, `$onehot`, `$onehot0`, `$countones`).
