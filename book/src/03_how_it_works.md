# Chapter 3: How VeloFormal Works Under the Hood

Understanding how VeloFormal transforms SystemVerilog into mathematical proofs helps you pick the right engine, debug tricky invariants, and scale verification to complex IP blocks.

## 3.1 The 5-Stage Compilation & Proof Pipeline

```text
SystemVerilog (.sv) + Contract (.vfc) / UPF / RDL / SDC
        │
        ▼  1. vf-slang (Embedded Slang v9.1 C++20)
           Full IEEE 1800 elaboration, parameter resolution, generate unrolling,
           SVA AST extraction, and optional AutoCheck/CoverCheck instrumentation
        │
        ▼  2. vf-ir (Word-Level Synchronous RTL Netlist IR)
           SSA-like hash-consed expression DAG (`ExprKind`), typed flip-flops (`Ff`),
           memories (`MemWrite`/`MemRead`), and `Property` nodes (Assert/Assume/Cover/Live/Fair)
        │
        ▼  3. IR Passes (vf-ir/src/passes.rs)
           • clk2fflogic  — multi-clock designs lowered to single global formal clock
           • async2sync   — asynchronous reset lowered to synchronous mux + initial constraint
           • chformal     — property role transformation (provider ↔ caller)
           • coi_prune    — exact Cone-of-Influence backward slicing
           • blackbox / cutpoint — assume-guarantee modular abstraction
        │
        ▼  4. Backends (vf-smt)
           • SMT-LIB2 (`QF_BV` / `QF_ABV`) — word-level bitvectors & arrays
           • BTOR2        — standard hardware model-checking interchange
           • Binary AIGER 1.0 — bit-blasted AIG for IC3/PDR
        │
        ▼  5. Proof Engines (vf-engine)
           BMC • k-Induction • IC3/PDR (ABC) • Parallel Portfolio • Case-Splitting
           • Fair Lasso Liveness • Cover • Vacuity • Dead-End • SLEC • Mutation
```

## 3.2 Bounded Model Checking (BMC)

BMC unrolls the transition relation $T(s_t, i_t, s_{t+1})$ for $t = 0, 1, \dots, D$:
$$I(s_0) \land \bigwedge_{t=0}^{k} A(s_t, i_t) \land \bigwedge_{t=0}^{k-1} T(s_t, i_t, s_{t+1}) \land \neg P(s_k)$$
- If **SAT**: the solver model yields a **shortest concrete counterexample trace** of length $k+1$.
- If **UNSAT** for all $k \le D$: no bug exists within $D$ cycles (bounded guarantee, not yet an unbounded proof).

## 3.3 k-Induction (Unbounded Word-Level Proof)

k-Induction proves $P$ holds **for all $t \ge 0$** by checking two formulas at increasing depth $k$:
1. **Base case**: $P$ holds for steps $0 \dots k$ starting from initial state $I(s_0)$.
2. **Inductive step**: Starting from an **arbitrary** state $s_0$ (dropping $I(s_0)$!), assume $P$ and assumptions $A$ hold for $k$ consecutive steps $s_0 \dots s_{k-1}$; prove $P$ must hold at $s_k$!

Because k-induction operates directly at the **word level** in SMT-LIB2, it proves wide 32-bit/64-bit arithmetic, datapath pipelines, and RISC-V ALU operations in milliseconds.

## 3.4 IC3 / PDR (Property-Directed Reachability)

Some invariants are **true** yet **not k-inductive** for any small $k$ (for example, a credit counter or FIFO where unreachable states violate an internal relationship between pointers and memory).

VeloFormal bit-blasts the COI-pruned word-level IR into binary **AIGER 1.0** and invokes **ABC IC3/PDR**, which automatically synthesizes an inductive strengthening invariant $F(s)$ such that:
$$I(s) \implies F(s), \qquad F(s) \land T(s, s') \implies F(s'), \qquad F(s) \implies P(s)$$
If PDR finds a counterexample, VeloFormal maps the bit-level witness back to full word-level RTL signals and generates a clean trace & backtrace.

## 3.5 Scaling Proofs: Portfolio, Case-Splitting, COI Cache & Modular Abstraction

- **Parallel Portfolio (`--engine portfolio`)**: Races word-level k-induction and bit-level ABC PDR concurrently on separate threads — the first winner returns immediately!
- **Parallel Case-Splitting (`--case-split mode=0..3`)**: Partitions the proof space across CPU cores by adding per-slice assumptions (`mode == 0`, `mode == 1`, ...) and proving every slice in parallel.
- **Deterministic COI Proof Cache (`--cache-dir .vf-cache`)**: Canonicalizes and SHA-256 hashes the COI-pruned IR so unchanged properties return instant cache hits across incremental RTL edits.
- **Blackboxing & Cutpoints (`--blackbox u_sub`, `--cutpoint sig`)**: Replace heavy submodules (e.g., multipliers or SRAMs) with unconstrained symbolic interfaces for assume-guarantee compositional verification.
