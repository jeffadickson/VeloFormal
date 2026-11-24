---
name: veloformal
description: Formally verify Verilog and SystemVerilog RTL using VeloFormal — inspect RTL structure, hunt zero-SVA bugs with AutoCheck, write and prove non-invasive .vfc contracts, verify RTL refactors via SLEC equivalence, run ISO 26262 fault and UPF power checks, and pass the 4-gate anti-cheating Trust Audit.
---

# VeloFormal — Formal Verification for the Human & Agentic Era

Use this skill whenever the user asks to formally verify, prove, bug-hunt, refactor safely, or audit Verilog/SystemVerilog hardware designs.

## Core Principles
1. **Inspect before specifying**: Always run `inspect_rtl` (MCP) or `veloformal inspect <rtl.sv> --format json` (CLI) to obtain exact signal names, widths, signedness, clock, and reset polarity.
2. **Prefer non-invasive `.vfc` contracts**: Use `veloformal scaffold <rtl.sv>` to generate a starter `.vfc` contract rather than editing production RTL with `` `ifdef FORMAL ``.
3. **Guard every RTL edit with SLEC**: When optimizing, pipelining, or clock-gating RTL, prove equivalence with `veloformal equiv --gold <orig.sv> --gate <new.sv>` (`--latency-gold`, `--reset`, `--valid-cond`, `--ignore-output`).
4. **Never ship vacuous or over-constrained proofs**: Always finish with `veloformal audit <rtl.sv> [--spec spec.vfc]` (or MCP `audit_trust`) to guarantee:
   - Unbounded safety proof (`PROVED`)
   - Zero assumption dead-ends (`deadend` clean)
   - Zero vacuous assertions (`vacuity` clean)
   - Real formal mutation kill-rate (use `surviving_mutants` to strengthen weak specs)

## Quick Command Reference

| Goal | CLI Command | MCP Tool |
|---|---|---|
| Introspect RTL ports/clocks/FSMs | `veloformal inspect dut.sv --format json` | `inspect_rtl` |
| Generate starter `.vfc` contract | `veloformal scaffold dut.sv -o dut.vfc` | `scaffold_contract` |
| Verify `.vfc` contract | `veloformal contract dut.sv --spec dut.vfc --vacuity --report signoff.html` | `verify_contract` |
| Zero-SVA RTL bug hunting | `veloformal autocheck dut.sv --fsm state` | `run_autocheck` |
| Prove RTL refactor equivalence | `veloformal equiv --gold g.sv --gate t.sv --reset rst=1` | `check_equiv` |
| Detect over-constrained assumptions | `veloformal deadend dut.sv` | `check_deadend` |
| ISO 26262 SA0/SA1/SEU fault analysis | `veloformal fault dut.sv --alarm err_flag --report fmeda.html` | `check_fault` |
| IEEE 1801 UPF power isolation check | `veloformal powercheck dut.sv --upf dut.upf` | `check_power` |
| AMBA APB / Raw CSR register check | `veloformal regcheck csr.sv --spec csr.rdl --protocol apb` | *(CLI)* |
| 4-Gate Anti-Cheating Trust Audit | `veloformal audit dut.sv --spec dut.vfc --format json` | `audit_trust` |

## `.vfc` v1.0 Contract Syntax Essentials
```vfc
contract ModuleSpec for my_rtl {
    clock posedge clk;
    reset when rst; // or: reset when !rst_n;

    require legal_env: !push || !full;
    invariant safe_bound: unsigned(count) <= 8;
    invariant idle_stable: !prev(en) => stable(out);

    operation write_op when push {
        ensure count_inc after 1 cycles:
            unsigned(count) == unsigned(at_trigger(count)) + 1;
    }

    cover reaches_full: full;
}
```
