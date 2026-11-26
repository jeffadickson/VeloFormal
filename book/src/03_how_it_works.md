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
        ▼  3. IR Passes (vf-ir/src/passes.rs & opt.rs)
           • clk2fflogic  — multi-clock designs lowered to single global formal clock
           • async2sync   — asynchronous reset lowered to synchronous mux + initial constraint
           • chformal     — property role transformation (provider ↔ caller)
           • coi_prune    — exact Cone-of-Influence backward slicing
           • sweep_sequential — sound constant-FF folding & duplicate-FF merging (with SLEC gold/gate matching)
           • blackbox / cutpoint — assume-guarantee modular abstraction
        │
        ▼  4. Backends (vf-smt)
           • SMT-LIB2 (`QF_BV` / `QF_ABV`) — O(N) Tseitin DAG-shared (`declare-fun` + `assert =`) word-level bitvectors & arrays
           • BTOR2        — standard hardware model-checking interchange
           • Binary AIGER 1.0 — bit-blasted AIG for ABC IC3/PDR & Craig interpolation
        │
        ▼  5. Proof Engines (vf-engine)
           BMC • k-Induction • ABC IC3/PDR (`dc2; scorr; dc2; pdr`) • ABC Craig Interpolation (`dc2; int`)
           • Per-Property Parallel COI Portfolio (+ Sound Lemma Promotion & Instant Cancellation)
           • Case-Splitting • Fair Lasso Liveness • Cover • Vacuity • Dead-End • SLEC • Mutation
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

Because k-induction operates directly at the **word level** in O(N) Tseitin DAG-shared SMT-LIB2 (`declare-fun` + `assert (= ...)` per shared sub-expression node, preserving E-graph sharing even across incremental `push`/`pop` queries), deep combinational diamond datapaths, 32-bit/64-bit arithmetic, and RISC-V ALU pipelines avoid exponential expression blowup and prove in milliseconds.

## 3.4 IC3 / PDR & Craig Interpolation (ABC Bit-Level Engines)

Some invariants are **true** yet **not k-inductive** for any small $k$ (for example, a credit counter or FIFO where unreachable states violate an internal relationship between pointers and memory).

VeloFormal bit-blasts the COI-pruned, sequentially-swept IR into binary **AIGER 1.0** and invokes **ABC** with two complementary unbounded engines:
1. **AIG-Simplified IC3/PDR (`strash; dc2; scorr -C 500; dc2; pdr`)**: Runs combinational AIG rewriting (`dc2`) and K-step sequential equivalence sweeping (`scorr`) before synthesizing an inductive strengthening invariant $F(s)$ such that:
$$I(s) \implies F(s), \qquad F(s) \land T(s, s') \implies F(s'), \qquad F(s) \implies P(s)$$
2. **Craig Interpolation (`strash; dc2; int -k`)**: Extracts over-approximating state-set interpolants from bounded UNSAT proofs — frequently closing proofs quickly on datapaths where PDR stalls.

If either engine finds a counterexample, VeloFormal maps the bit-level witness back to full word-level RTL signals and generates a clean trace & backtrace.

## 3.5 Scaling to Large Designs: Per-Property COI, Sequential Sweeping, Portfolio & Cancellation

VeloFormal combines six orthogonal scaling layers for multi-block SoC subsystems:

- **O(N) SMT-LIB2 Tseitin DAG Sharing (`declare-fun` + `assert =`)**: Every shared combinational IR node is emitted once per step/query as a named auxiliary constant, preventing exponential string blowup and avoiding incremental `define-fun` macro-expansion overhead in Z3/Bitwuzla/CVC5.
- **Sequential Constant Propagation & Duplicate-FF Sweeping (`sweep_sequential`)**: Soundly collapses initialized constant-preserving registers and merges structurally identical initialized FFs (including unchanged gold/gate register pairs in SLEC miters) while leaving uninitialized FFs untouched for X-Check and fault injection.
- **Per-Property Parallel COI Decomposition (`--engine portfolio`)**: When a module contains multiple assertions, VeloFormal automatically slices each assertion onto its minimal Cone-of-Influence, sweeps redundant FFs per slice, and proves assertions concurrently across CPU cores. Any assertion proved independently can be soundly promoted to an `Assume` helper lemma if a sibling assertion requires mutual induction.
- **3-Engine Portfolio + Instant Subprocess Cancellation (`CancelToken`)**: Each proof obligation races word-level k-induction, ABC `dc2+scorr+pdr`, and ABC `dc2+int` concurrently. The instant any worker proves or falsifies a property, `CancelToken` kills and reaps every losing `z3`/`yosys-abc` subprocess in < 1 ms with zero zombie processes.
- **Parallel Case-Splitting (`--case-split mode=0..3`)**: Partitions the proof space across CPU cores by adding per-slice assumptions (`mode == 0`, `mode == 1`, ...) and proving every slice in parallel.
- **Deterministic COI Proof Cache (`--cache-dir .vf-cache`) & Modular Abstraction (`--blackbox`, `--cutpoint`)**: SHA-256 COI caching skips unchanged properties across incremental edits, while blackboxing & cutpoints enable assume-guarantee verification of large hierarchies.
