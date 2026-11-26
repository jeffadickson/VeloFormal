# VeloFormal

**Formal verification for the human and agentic era.**

Native Verilog & SystemVerilog formal verification suite written in Rust — combining an embedded [Slang](https://github.com/MikePopoloski/slang) IEEE 1800 frontend, word-level RTL netlist IR, SMT-LIB2 / BTOR2 / AIGER backends, full SymbiYosys & Questa-class Formal Apps, the human-readable `.vfc` v1.0 Contract DSL, a VS Code Language Server (`vf-lsp`), and a native **Model Context Protocol server (`vf-mcp`)** with an **anti-cheating Trust Audit** (`veloformal audit`) so humans and AI coding agents collaborate on mathematically verified silicon.

- **Read the Book**: [***VeloFormal: Formal Verification for the Human & Agentic Era***](book/src/SUMMARY.md) (10 chapters on the Why, the How, Formal Apps, SLEC, ISO 26262, UPF & the Agentic Era)
- **Full Language Spec**: [`.vfc` v1.0 Contract Language Specification](docs/VFC_SPEC.md) (complete EBNF grammar, strict type system, formal semantics & SVA translation)
- **DSL vs. SVA Showcase**: [3-Tier Side-by-Side Comparison (`handshake_pipe` → `packet_fifo` → `secure_crypto_engine`)](examples/22_dsl_vs_sva_showcase/README.md)
- **Agentic Setup**: [`docs/AGENTIC_FORMAL.md`](docs/AGENTIC_FORMAL.md) • [`AGENTS.md`](AGENTS.md) • [`skills/veloformal/SKILL.md`](skills/veloformal/SKILL.md)

> The previous C#/Python prototype is preserved on the `archive/csharp-prototype` branch and `archive-csharp-prototype` tag.

## Architecture

```text
Verilog / SystemVerilog (.v / .sv / .sby / .vfc / .rdl / .xml / .sdc / .upf / .do)
  │
  ▼  vf-slang   — Embedded Slang v9.1 C++20 frontend + IEEE 1800 SVA (`bind`, repetition, `until`) & RTL instrumentation
  ▼  vf-ir      — Word-level RTL netlist IR + passes (opt, sweep_sequential, coi_prune, chformal, async2sync, clk2fflogic)
  ▼  vf-smt     — O(N) DAG-shared SMT-LIB2 (QF_BV/QF_ABV), BTOR2, binary AIGER 1.0 bit-blaster, cancellable solver pipe, VCD writer
  ▼  vf-engine  — BMC, k-Induction, ABC IC3/PDR (`dc2+scorr+pdr`) & Craig Interpolation (`dc2+int`), Per-Property Parallel COI Portfolio (+ Lemma Promotion), Liveness/Fairness, Cover, Vacuity, Dead-End, Backtrace, SLEC
  ▼  vf-apps    — Formal Apps: AutoCheck, CoverCheck, Connectivity, Secure (IFT), X-Check, Register Check (CSV/RDL/IP-XACT/APB), CDC/RDC (SDC), Fault (ISO 26262), PowerCheck (UPF), Agent Introspection & Trust Audit
  ▼  vf-dsl     — `.vfc` v1.0 Contract DSL (brace syntax, strict typing, native IR lowering & IEEE 1800 SVA `bind` export)
  ▼  vf-lsp     — Native Rust Language Server + VS Code extension (`editors/vscode/`)
  ▼  vf-mcp     — Native Rust Model Context Protocol (MCP) server for Claude, Cursor, Windsurf, Codex & Antigravity
  ▼  vf-cli     — `veloformal` CLI (`inspect`, `scaffold`, `audit`, `.sby`, Questa `.do` Tcl, Interactive HTML/SVG Waveform + Backtrace & SARIF/JUnit/JSON)
  └── vip/      — Reusable Formal Assertion & QFL Library (Ready/Valid, AXI4-Lite, AXI4-Stream, APB, AHB-Lite, Wishbone, TileLink-UL, FIFO, Arbiter, Gray-Code)
```

## Prerequisites

- Rust toolchain (`cargo`, `rustc`)
- C++20 compiler (`g++` 11+ / `clang++` 14+ / MSVC 2022) + `cmake` 3.20+ (plus `ninja` recommended)
- An SMT-LIB2 solver on `PATH`: `z3` (default), `bitwuzla`, or `cvc5`
- Optional: `yosys-abc` (or `abc`) for IC3/PDR proofs (`--engine pdr` / `abc pdr` in `.sby`)

## Build & Test

```bash
git submodule update --init --recursive
cargo build --release --workspace
cargo test --workspace
```

## Formal for the Human & Agentic Era (`vf-mcp`, `inspect`, `scaffold`, `audit`)

VeloFormal treats AI coding agents as first-class collaborators alongside human verification engineers — with built-in anti-cheating guardrails (see [`docs/AGENTIC_FORMAL.md`](docs/AGENTIC_FORMAL.md), [`AGENTS.md`](AGENTS.md), and [`skills/veloformal/SKILL.md`](skills/veloformal/SKILL.md)):

```bash
# 1. Machine-readable RTL introspection (ports, widths, clock/reset polarity, FSMs, protocols)
veloformal inspect examples/21_questa_parity/apb_csr.sv --format json

# 2. Auto-scaffold a valid, type-checked starter `.vfc` contract from RTL
veloformal scaffold examples/18_vfc_contracts/fifo_rtl.sv -o fifo_starter.vfc

# 3. 4-Gate Anti-Cheating Trustworthiness Audit (Proof + Dead-End + Vacuity + Mutation Score)
veloformal audit examples/18_vfc_contracts/counter.sv \
    --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --format json

# 4. Native Rust MCP Server (10 typed tools + `.vfc` spec resource + verification prompts)
cargo build --release --bin vf-mcp
```

## VeloFormal Contract DSL (`.vfc` v1.0)

Write clean, non-invasive Eiffel-style RTL contracts (zero `` `ifdef FORMAL `` edits to production RTL) with strict width/signedness checking, automatic post-reset history guards, symbolic/ghost scoreboards, reusable protocol `use` clauses, embedded `xfree`/`secure` checks, and portable IEEE 1800 SVA `bind` export:

```bash
# Prove, cover, check vacuity, and generate an Interactive HTML Sign-Off Dashboard + SVG waveforms
veloformal contract examples/18_vfc_contracts/counter.sv \
    --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --vacuity --report signoff.html

# Verify a pure-RTL FIFO end-to-end with `use fifo_scoreboard(...)`
veloformal contract examples/18_vfc_contracts/fifo_rtl.sv \
    --spec examples/18_vfc_contracts/fifo_contract.vfc

# Export portable IEEE 1800 SVA `bind` module for interop with Questa / JasperGold / Verilator
veloformal emit-sva examples/18_vfc_contracts/counter.vfc \
    --rtl examples/18_vfc_contracts/counter.sv -o counter_sva.sv

# Canonical 4-space LF formatter
veloformal fmt examples/18_vfc_contracts/counter.vfc --check
```

## Core Formal Verification (SymbiYosys / PropCheck / SLEC / Questa Tcl)

```bash
# Bounded Model Checking (BMC) with Root-Cause Backtrace, skip, append, and VCD counterexample
veloformal bmc tests/verilog/counter_fail.sv --depth 10 --append 2 --vcd counter_fail.vcd

# Unbounded safety proofs: k-induction, IC3/PDR, or portfolio, with optional COI pruning
veloformal prove tests/verilog/sync_fifo.sv --engine kind
veloformal prove tests/verilog/pdr_non_inductive.sv --engine pdr --coi
veloformal prove examples/19_industry_standards/sva_bind_and_rep.sv --top pulse_stretcher

# Assertion vacuity & over-constraint dead-end detection (Questa PropCheck)
veloformal vacuity tests/verilog/vacuity_test.sv
veloformal deadend examples/21_questa_parity/overconstrained_fifo.sv

# Liveness & fairness checking (bounded fair lasso detection)
veloformal live tests/verilog/liveness_starve.sv --depth 10 --vcd lasso.vcd

# Cover reachability witnesses (writes cover_0.vcd, ...)
veloformal cover tests/verilog/sync_fifo.sv --depth 10 --append 1 --vcd-dir waves/

# Full SymbiYosys (.sby) multi-task runner ([tasks], [file ...], portfolio engines, workdir status/VCD)
veloformal sby tests/verilog/multi_task.sby -d workdir/

# Questa-Formal-compatible Tcl (.do) runner with SARIF / JUnit / JSON / Interactive HTML Dashboard
veloformal tcl examples/19_industry_standards/questa_signoff.do --format sarif --report signoff.html
veloformal tcl examples/21_questa_parity/run_questa_parity.do --report parity_signoff.html

# Sequential Logic Equivalence Checking (SLEC): pipeline latency, reset seq, LPGC valid-cond, safety-insertion output filtering
veloformal equiv --gold tests/verilog/adder_gold.sv --gate tests/verilog/adder_gate.sv
veloformal equiv --gold tests/verilog/slec_pipe_gold.sv --gate tests/verilog/slec_pipe_gate.sv --latency-gold 2
veloformal equiv --gold examples/21_questa_parity/lpgc_gold.sv --gate examples/21_questa_parity/lpgc_gate.sv \
    --reset rst=1 --valid-cond out_valid --ignore-output cg_active

# Export IR, BTOR2, SMT-LIB2, or binary AIGER
veloformal dump-ir tests/verilog/sync_fifo.sv
veloformal dump-btor2 tests/verilog/sync_fifo.sv
veloformal dump-smt2 tests/verilog/sync_fifo.sv
veloformal dump-aiger tests/verilog/pdr_non_inductive.sv -o pdr.aig
```

## Formal Apps (Questa One / Questa Formal Suite Equivalents)

```bash
# AutoCheck: push-button RTL bug hunting (OOB memory, div-by-zero, shift-overflow, unique/priority, overflow, FSM deadlock/livelock/unreachable, clock-as-data, const-reg)
veloformal autocheck examples/21_questa_parity/fsm_livelock.sv --fsm state --check-const-regs --vcd-dir waves/

# Analyze Fault (ISO 26262): formal SA0/SA1/SEU fault injection, Safe/Masked vs Detected vs Dangerous SPF, SPFM & Diagnostic Coverage
veloformal fault examples/21_questa_parity/tmr_voter.sv --target r0 --output safe_out
veloformal fault examples/21_questa_parity/parity_reg.sv --target data_reg --alarm parity_err --report fmeda.html

# PowerCheck (IEEE 1801 UPF): formal power-domain isolation clamp (0 / 1 / latch), power-off sequencing & un-isolated crossing detection
veloformal powercheck examples/21_questa_parity/upf_iso_dut.sv --upf examples/21_questa_parity/upf_iso.upf --pd-off PD_ACCEL=pd_off

# CoverCheck: automatic branch reachability witnesses & unbounded dead-code proofs
veloformal covercheck tests/verilog/covercheck_dead.sv

# Connectivity Check: static/conditional/inverted/delayed pin-to-pin routing rules from CSV
veloformal connectivity tests/verilog/mux_crossbar.sv --spec rules.csv

# Secure Check: formal Information Flow Tracking (2-copy self-composition + optional declassification)
veloformal secure tests/verilog/secure_crypto.sv --source secret_key --target pub_out --declassify debug_auth

# X-Check: 4-state dual-rail formal X-propagation verification (controlling values & uninitialized FFs)
veloformal xcheck tests/verilog/xcheck_uninit.sv --target out_leak --target out_masked

# Register Check: CSR reset, RW/RO/W1C/W1S/WO, masks & non-aliasing (CSV, SystemRDL .rdl, IP-XACT .xml; Raw or AMBA APB; 0/1-cycle read latency)
veloformal regcheck examples/15_regcheck_uart_csr/uart_csr.sv --spec examples/19_industry_standards/uart_csr.rdl
veloformal regcheck examples/21_questa_parity/apb_csr.sv --spec examples/21_questa_parity/apb_csr.rdl --protocol apb

# CDC / RDC: structural clock/reset-domain crossing analyzer + SDC constraints + formal Gray-code verification
veloformal cdc examples/16_cdc_async_bridge/async_bridge.sv --gray wr_ptr_gray
veloformal cdc examples/16_cdc_async_bridge/async_bridge_buggy.sv --sdc examples/19_industry_standards/async_bridge.sdc
```

## Formal Assertion & QFL Library (`vip/`)

Reusable SystemVerilog formal protocol & QFL checkers included in `vip/`:

- [`vip/vf_handshake.sv`](vip/vf_handshake.sv) — Ready/Valid stability under backpressure + transfer covers
- [`vip/vf_axi4_lite.sv`](vip/vf_axi4_lite.sv) — AMBA AXI4-Lite AW/W/B/AR/R channel compliance
- [`vip/vf_axis.sv`](vip/vf_axis.sv) — AMBA AXI4-Stream (`tvalid`/`tready`/`tdata`/`tkeep`/`tlast`)
- [`vip/vf_apb.sv`](vip/vf_apb.sv) — AMBA APB SETUP/ACCESS phase & wait-state stability
- [`vip/vf_ahb_lite.sv`](vip/vf_ahb_lite.sv) — AMBA AHB-Lite wait-state & transfer stability
- [`vip/vf_wishbone.sv`](vip/vf_wishbone.sv) — Wishbone B4 pipelined/classic protocol checker
- [`vip/vf_tilelink_ul.sv`](vip/vf_tilelink_ul.sv) — SiFive TileLink-UL Channel A/D checker
- [`vip/vf_fifo_props.sv`](vip/vf_fifo_props.sv) — Reusable FIFO safety & `$anyconst` data-integrity scoreboard
- [`vip/vf_arbiter_props.sv`](vip/vf_arbiter_props.sv) — QFL Arbiter mutual-exclusion, no-spurious-grant & work-conserving checker
- [`vip/vf_gray_code.sv`](vip/vf_gray_code.sv) — QFL Gray-code single-bit transition checker (`$countones(g ^ $past(g)) <= 1`)

## Proof Scalability, Formal Sign-Off Coverage & IDE (`vf-lsp`)

```bash
# Parallel portfolio (races k-ind + ABC PDR), parallel case-splitting, COI cache & blackboxing
veloformal prove tests/verilog/pdr_non_inductive.sv --engine portfolio --cache-dir .vf-cache
veloformal prove examples/01_traffic_light_fsm/traffic_light.sv --case-split ew_car=0..1

# Proof Core (COI signal coverage + minimal assumption core) & Formal Mutation Testing (%)
veloformal proof-core examples/06_arbiter_liveness/rr_arbiter_4.sv
veloformal mutate examples/18_vfc_contracts/counter.sv --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --report mutate.html

# Integrity-hashed Proof Certificates for assume-guarantee composition
veloformal contract examples/18_vfc_contracts/counter.sv --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --emit-cert counter.vfcert.json

# Real 2-stage pipelined RV32I RISC-V ISA formal verification + X-Check
veloformal contract examples/20_riscv_rv32i_formal/rv32i_core.sv --spec examples/20_riscv_rv32i_formal/rv32i_isa.vfc --report rv32i.html

# Native Rust Language Server + VS Code extension (`editors/vscode/`)
cargo build --release --bin vf-lsp

# Batteries-included Docker container (`veloformal` + `vf-lsp` + `z3` + `yosys-abc`)
docker build -t veloformal .
```

## Examples

See [`examples/README.md`](examples/README.md) for **20 self-contained, CI-verified formal verification examples** covering a pipelined RV32I RISC-V core, `.vfc` contracts, IEEE 1800 SVA (`bind`, named properties, repetition, `until`), SystemRDL/IP-XACT/SDC/Questa Tcl, FSMs, skid buffers, FIFOs, AMBA AXI4-Lite/APB VIP, round-robin arbitration & starvation lassos, IC3/PDR, SLEC, AutoCheck, CoverCheck, Connectivity, Secure IFT, X-Check, Register Check, CDC/RDC, and Vacuity detection.

## SymbiYosys & SystemVerilog Formal Coverage

- **`.sby` Runner**: `[tasks]` with tag filtering (`tag:`, `~tag:`), inline `[file <name>]` blocks, `[options]` (`mode bmc|prove|cover|live`, `depth`, `skip`, `append`, `expect`, `multiclock`), `[engines]` (`smtbmc z3|bitwuzla|cvc5`, `abc pdr` with parallel portfolio racing), `[script]` (`read`/`read_verilog`, `prep`, `chparam`, `chformal`, `async2sync`, `clk2fflogic`), and SBY-compatible `<workdir>/status` + `engine_0/*.vcd` outputs.
- **RTL & IEEE 1800 SVA**: Full preprocessing (`` `ifdef FORMAL ``), hierarchy flattening, `bind` directives, `generate` loops, continuous `assign`, `always_comb`/`always_ff`/`initial`, `if`/`else`, `case`/`casez`/`casex` wildcards, `unique`/`priority` qualifiers, procedural `for`/`repeat` loop unrolling, packed `struct` fields, pure SV `function` inlining, unpacked 1D memories (SMT `QF_ABV` arrays + bit-blasted AIGER), async resets (`async2sync`), multi-clock (`clk2fflogic`), named `property`/`sequence`, immediate & concurrent SVA (`|->`, `|=>`, `##N`, `##[M:N]`, `[*N]`, `[*M:N]`, `[->N]`, `[=N]`, `until`, `s_until`, `until_with`, `throughout`, `first_match`, `iff`, `if`/`else`, `disable iff`, `s_eventually`), labeled assertions, Yosys attributes (`(* anyseq *)`, `(* anyconst *)`, `(* gclk *)`), and system functions (`$past`, `$rose`, `$fell`, `$stable`, `$changed`, `$initstate`, `$anyseq`, `$anyconst`, `$onehot`, `$onehot0`, `$countones`).
