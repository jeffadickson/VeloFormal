# Preface — Formal for the Human & Agentic Era

Hardware verification is entering a once-in-a-generation inflection point.

For the past 30 years, formal verification sat behind two formidable walls:
1. **A human usability wall**: cryptic SystemVerilog Assertion (SVA) syntax, fragile `$past` reset guards, manual `$anyconst` scoreboard state machines, and million-dollar proprietary GUI toolchains.
2. **An agentic usability wall**: AI coding agents can now write RTL at 10x speed, yet traditional EDA tools give agents opaque Tcl logs, 50 MB VCD dumps, and zero guardrails against reward-hacked vacuous or over-constrained proofs.

**VeloFormal** tears down both walls in a single native Rust workspace:
- **For Humans**: a crisp, strictly typed `.vfc` v1.0 Contract DSL, live VS Code LSP diagnostics, Interactive HTML Sign-Off Dashboards with embedded SVG waveforms and Root-Cause Backtraces, and push-button Questa-class Formal Apps.
- **For AI Agents**: a native Rust **Model Context Protocol server (`vf-mcp`)**, JSON RTL introspection (`veloformal inspect`), contract scaffolding (`veloformal scaffold`), token-efficient COI Markdown trace tables, SLEC refactoring guardrails, and the **4-Gate Anti-Cheating Trust Audit** (`veloformal audit`).
- **Zero Vendor Lock-In**: full embedded IEEE 1800 SystemVerilog via Slang v9.1, SymbiYosys (`.sby`) execution, Questa Tcl (`.do`) execution, SystemRDL / IP-XACT / SDC / UPF ingestion, and portable IEEE 1800 SVA `bind` export (`veloformal emit-sva`).

This book explains **why** formal verification is essential in the era of AI-assisted silicon design, **how** the underlying mathematical engines work, and **how** humans and agents can verify real hardware end-to-end with VeloFormal.
