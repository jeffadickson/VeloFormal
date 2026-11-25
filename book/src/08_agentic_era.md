# Chapter 8: Formal Verification in the Agentic Era

AI coding agents are transforming RTL engineering — drafting pipelines, protocol bridges, CSR banks, and microarchitectural optimizations in minutes. Yet raw speed without mathematical proof only ships subtler bugs faster.

VeloFormal provides the complete infrastructure for **trustworthy human + agent collaboration**.

## 8.1 Why Agents Need Dedicated Formal Infrastructure

When an LLM tries to drive a legacy EDA tool via bash commands, four failure modes dominate:
1. **Hallucinated Ports & Polarities**: Guessing `rst_n` vs `rst` or 7-bit vs 8-bit widths wastes iterations.
2. **Context Explosion on Failures**: Dumping a 200 KB VCD file into an LLM prompt blows out context windows and obscures the bug.
3. **SVA Boilerplate Bugs**: Hand-coding `f_past_valid` and `$anyconst` scoreboards in SVA causes frequent syntax and off-by-one errors.
4. **Reward Hacking / Cheating**: Under pressure to "make all assertions pass", agents sometimes weaken assertions to `assert(1)`, write vacuous implications whose antecedent is unreachable, or add contradictory `assume`s that create a dead-end state at cycle 2!

## 8.2 Native Rust MCP Server (`vf-mcp`)

Build and register `vf-mcp` in Claude Code, Claude Desktop, Cursor, Windsurf, or Antigravity (`mcp.json`):

```json
{
  "mcpServers": {
    "veloformal": {
      "command": "/path/to/target/release/vf-mcp",
      "args": []
    }
  }
}
```

`vf-mcp` exposes:
- **10 Typed Formal Tools**: `inspect_rtl`, `scaffold_contract`, `verify_contract` (supports inline `contract_text`!), `prove_sv`, `run_autocheck`, `check_equiv`, `check_deadend`, `check_fault`, `check_power`, and `audit_trust`.
- **Compact Agent Counterexamples**: Failures return a COI `backtrace` plus a concise Markdown `trace_table` (~300 tokens instead of 50,000 tokens of raw VCD!).
- **Live Knowledge Resources**: `veloformal://vfc-spec` and `veloformal://workflows` so connected agents immediately write valid `.vfc` v1.0 syntax.
- **Built-In Prompts**: `formal-verify-rtl` and `verify-rtl-refactor`.

## 8.3 The 4-Gate Anti-Cheating Trust Audit (`veloformal audit`)

Never accept a formal proof — from an agent or a human — without passing the 4-Gate Trust Audit:

```bash
veloformal audit examples/18_vfc_contracts/counter.sv \
    --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --format json
```

| Gate | Engine | What Cheating / Bug Pattern It Catches |
|---|---|---|
| **Gate 1 — Safety Proof** | Parallel Portfolio (`k-ind` + ABC PDR) | Failing assertions, missing assertions (`assert_count == 0`), or `ASSUMPTIONS_UNSAT` |
| **Gate 2 — Dead-End Check** | `check_deadends` (quantified SMT) | Over-constrained `assume`s that reach a state where NO next-cycle input satisfies `assume` |
| **Gate 3 — Vacuity Check** | `check_vacuity` | Assertions whose guard/antecedent is unreachable (e.g., `(cnt == 15) => ...` when `cnt <= 8`) |
| **Gate 4 — Mutation Score** | Parallel RTL Mutation Testing | Weak/trivial specs (e.g., `assert(1)`) that survive injected RTL bugs — returns exact `surviving_mutants` so the agent knows what edge case to specify next! |

Only when all four gates pass does `veloformal audit` emit `trustworthy: true` (`AUDIT TRUSTWORTHY`).

## 8.4 The Golden Human + Agent Loop

1. **Inspect**: `inspect_rtl` / `veloformal inspect dut.sv --format json`
2. **Hunt**: `run_autocheck` / `veloformal autocheck dut.sv --fsm state`
3. **Specify**: `scaffold_contract` → refine `.vfc` contract → `verify_contract`
4. **Refactor Safely**: `check_equiv` / `veloformal equiv --gold orig.sv --gate opt.sv`
5. **Audit & Sign Off**: `audit_trust` / `veloformal audit dut.sv --spec dut.vfc` + emit HTML dashboard (`--report signoff.html`) and SHA-256 `ProofCertificate` (`--emit-cert`).
