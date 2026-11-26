# AGENTS.md — VeloFormal Guide for AI Coding Agents

VeloFormal is **formal verification built for the human and agentic era**: embedded Slang IEEE 1800 SystemVerilog frontend, readable `.vfc` v1.0 contracts, Questa-class Formal Apps, and a native Rust MCP server (`vf-mcp`).

## Preferred Tooling (MCP vs CLI)

If the `vf-mcp` Model Context Protocol server is connected, prefer its typed JSON tools (`inspect_rtl`, `scaffold_contract`, `verify_contract`, `prove_sv`, `run_autocheck`, `check_equiv`, `check_deadend`, `check_fault`, `check_power`, `audit_trust`, `compose_verify`, `get_vip_template`) and read resource `veloformal://vfc-spec`.

When working in a terminal, use `veloformal` directly with `--format json` where supported.

## Golden Verification Loop (Never Hallucinate, Never Cheat)

1. **Inspect RTL first** — never guess port names, widths, clock names, or reset polarity:
   ```bash
   veloformal inspect <rtl.sv> --format json
   ```
2. **Run zero-SVA bug hunting** before writing specs:
   ```bash
   veloformal autocheck <rtl.sv> --auto-fsm [--strict] --format json
   ```
3. **Scaffold & refine a non-invasive `.vfc` contract** (or start from a turnkey Protocol VIP via `veloformal vip init <proto>`) — do NOT pollute production RTL with `` `ifdef FORMAL `` unless explicitly asked:
   ```bash
   veloformal scaffold <rtl.sv> -o spec.vfc
   veloformal vip init axi4_lite --module <mod> -o axi.vfc
   veloformal contract <rtl.sv> --spec spec.vfc --vacuity
   ```
4. **Scale to multi-block SoCs via Automated Assume-Guarantee Composition** (blackboxing verified leaf submodules):
   ```bash
   veloformal compose <leaf.sv> <top.sv> --top <top> --spec top.vfc --sub u_leaf=<leaf.sv>:<leaf_mod>:<leaf.vfc> --format json
   ```
5. **Prove every RTL refactor / optimization / clock-gating change via SLEC**:
   ```bash
   veloformal equiv --gold orig.sv --gate refactored.sv [--reset rst=1] [--latency-gold N] [--valid-cond out_valid] [--ignore-output extra_port]
   ```
6. **Pass the 4-Gate Anti-Cheating Trust Audit (and/or `vplan.json`) before claiming sign-off**:
   ```bash
   veloformal audit <rtl.sv> --spec spec.vfc --format json
   veloformal vplan soc_vplan.json --format json
   ```
   A proof is **NOT** complete unless `trustworthy == true`:
   - **Gate 1 (Safety Proof)**: Unbounded proof passes (`PROVED`).
   - **Gate 2 (Dead-End Check)**: Assumptions never reach a dead-end state (`CLEAN`).
   - **Gate 3 (Vacuity Check)**: Every assertion antecedent is reachable (`NON-VACUOUS`).
   - **Gate 4 (Mutation Score)**: Formal properties actually kill injected RTL bugs (inspect `surviving_mutants` to close spec gaps).

## Debugging Counterexamples

- Use the compact COI **Root-Cause Backtrace** and **Markdown cycle table** (`trace_table`) returned on failure instead of dumping raw VCD files into context.
- Only generate `.vcd` or `--report signoff.html` (interactive SVG waveform + synchronized clickable COI backtrace) when requested by the human reviewer.

## Repository Development Commands

```bash
cargo build --workspace
cargo test --workspace
```
