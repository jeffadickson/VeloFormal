# Formal Verification for the Human & Agentic Era

Traditional formal verification tools were built exclusively for specialist verification engineers clicking through closed GUIs with opaque TCL scripts and 50 MB VCD waveforms.

**VeloFormal** is designed from the ground up so **humans and AI coding agents collaborate on mathematically verified silicon**:

- **For Humans**: Readable Eiffel-style `.vfc` contracts, live VS Code LSP (`vf-lsp`), Interactive HTML Sign-Off Dashboards with embedded SVG waveforms + Root-Cause Backtraces, and drop-in Questa/SymbiYosys compatibility.
- **For Agents**: Native Rust **MCP Server** (`vf-mcp`), structured JSON RTL introspection (`inspect`), contract scaffolding (`scaffold`), token-efficient COI Markdown trace tables, SLEC equivalence guardrails for AI RTL refactors, and the **4-Gate Anti-Cheating Trust Audit** (`audit`).

---

## 1. Connecting `vf-mcp` to Your AI Coding Agent

Build the MCP server binary:

```bash
cargo build --release --bin vf-mcp --bin veloformal
```

### Claude Code / Claude Desktop / Cursor / Windsurf / Antigravity (`mcp.json`)

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

Once connected, the agent automatically gains:
- **10 Formal Tools**: `inspect_rtl`, `scaffold_contract`, `verify_contract`, `prove_sv`, `run_autocheck`, `check_equiv`, `check_deadend`, `check_fault`, `check_power`, `audit_trust`
- **2 Live Knowledge Resources**: `veloformal://vfc-spec` (exact `.vfc` grammar) and `veloformal://workflows`
- **2 Built-In Prompts**: `formal-verify-rtl` and `verify-rtl-refactor`

---

## 2. The Anti-Cheating Trustworthiness Audit (`veloformal audit`)

A known failure mode of AI agents writing formal proofs is **reward hacking**: weakening assertions to `assert(1)`, writing vacuous implications whose antecedent never triggers, or adding contradictory `assume`s that silently hide every bug after cycle 2.

`veloformal audit` (and MCP `audit_trust`) enforces a strict **4-Gate Formal Trustworthiness Sign-Off**:

```bash
veloformal audit examples/18_vfc_contracts/counter.sv \
    --spec examples/18_vfc_contracts/counter.vfc -G MAX_VAL=5 --format json
```

1. **Gate 1 — Unbounded Safety Proof**: Races k-induction + ABC IC3/PDR to prove all assertions hold for all time.
2. **Gate 2 — Dead-End / Over-Constraint Check**: Proves `assume` constraints never reach a dead-end state where no valid next-cycle input exists.
3. **Gate 3 — Vacuity Check**: Proves every assertion's guard/antecedent is actually reachable.
4. **Gate 4 — Formal Mutation Testing**: Injects real RTL bugs into the Cone-of-Influence and verifies the specification kills them (returning `surviving_mutants` so the agent knows exactly what edge case to specify next!).

---

## 3. SLEC Guardrail for Autonomous RTL Refactoring

When an AI agent optimizes RTL (retiming, pipelining, low-power clock gating, or adding TMR/ECC safety logic), pair it with `check_equiv` / `veloformal equiv`:

```bash
veloformal equiv \
    --gold examples/21_questa_parity/lpgc_gold.sv \
    --gate examples/21_questa_parity/lpgc_gate.sv \
    --reset rst=1 --valid-cond out_valid --ignore-output cg_active
```

If a single edge case diverges on any cycle, VeloFormal returns a minimal COI **Markdown cycle table** so the agent can immediately pinpoint and fix the bug.
