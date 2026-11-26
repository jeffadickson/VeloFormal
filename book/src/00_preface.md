# Preface — Formal for the Human & Agentic Era

Hardware verification is entering a once-in-a-generation inflection point.

For the past 30 years, formal verification sat behind two formidable walls:
1. **A human usability wall**: cryptic SystemVerilog Assertion (SVA) syntax, fragile `$past` reset guards, manual `$anyconst` scoreboard state machines, and million-dollar proprietary GUI toolchains.
2. **An agentic usability wall**: AI coding agents can now write RTL at 10x speed, yet traditional EDA tools give agents opaque Tcl logs, 50 MB VCD dumps, and zero guardrails against reward-hacked vacuous or over-constrained proofs.

**VeloFormal** tears down both walls in a single native Rust workspace:
- **For Humans**: a crisp, strictly typed `.vfc` v1.0 Contract DSL, 8 turnkey bus/ISA Protocol VIPs (`veloformal vip`), Traceable Formal Verification Plans (`veloformal vplan`), live VS Code LSP diagnostics & embedded Webview Waveform Debugger, Interactive HTML Sign-Off Dashboards with synchronized COI Root-Cause Backtraces, and push-button 18-check AutoCheck & Questa-class Formal Apps.
- **For Chip-Scale SoCs**: O(N) DAG-shared SMT2, sequential COI sweeping, parallel per-property lemma promotion, ABC AIG IC3/PDR, word-level BTOR2 (`pono`/`avr`/`btormc`), and **Automated Hierarchical Assume-Guarantee Composition** (`veloformal compose`).
- **For AI Agents**: a native Rust **Model Context Protocol server (`vf-mcp`)** with 12 typed JSON tools, RTL introspection (`veloformal inspect`), contract scaffolding (`veloformal scaffold`), token-efficient COI Markdown trace tables, SLEC refactoring guardrails, and the **4-Gate Anti-Cheating Trust Audit** (`veloformal audit`).
- **Zero Vendor Lock-In**: full embedded IEEE 1800 SystemVerilog via Slang v9.1 (OpenTitan structs, 2D packed arrays, `interface`/`modport`, instance arrays, streaming operators, `case inside`), SymbiYosys (`.sby`) & Questa Tcl (`.do`) execution, SystemRDL / IP-XACT / SDC / UPF ingestion, portable IEEE 1800 SVA `bind` export (`veloformal emit-sva`), and a first-class GitHub Action (`action.yml`) + Homebrew packaging.

Across **14 hands-on chapters**, this book explains **why** formal verification is essential in the era of AI-assisted silicon design, **how** the underlying mathematical engines scale to real chips, and **how** humans and agents can verify production hardware end-to-end with VeloFormal.
