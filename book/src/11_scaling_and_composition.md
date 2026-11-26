# Chapter 11: Chip-Scale Formal — Scaling Optimizations, Word-Level BTOR2 & Assume-Guarantee Composition

Writing a formal contract for a 200-line FIFO is easy. Proving safety invariants on a 50,000-gate SoC subsystem with 64-bit datapaths, multi-stage pipelines, and nested sub-blocks is where toy formal tools hit a wall — and where VeloFormal's **four-layer chip-scale architecture** takes over.

---

## 1. Layer 1: Linear-Size IR & DAG-Shared Encoding

Naive formal frontends inline combinational expressions when unrolling time frames, causing exponential formula blowup ($O(2^N)$) on deep arithmetic chains, priority encoders, and crossbars.

VeloFormal eliminates formula blowup before the solver ever sees a single clause:

1. **O(N) Tseitin DAG Sharing in SMT2 (`vf-smt`)**: Every shared combinational `ExprId` is bound once per time frame via `(declare-fun ...)` + `(assert (= ...))`, keeping the SMT2 size strictly linear in the circuit gate count.
2. **Sequential Structural & COI Sweeping (`sweep_sequential`)**: Before encoding, VeloFormal computes the exact backward **Cone of Influence (COI)** from all active assertions, assumptions, and covers — pruning dead flip-flops, unused CSR banks, and unrelated debug counters.
3. **Per-Property Parallel COI Decomposition + Lemma Promotion**: When a contract has many invariants, VeloFormal can prove each invariant in its own minimal COI in parallel. Once an invariant is proved unboundedly, it is **promoted into an inductive lemma** (assumption) to help harder properties converge faster.

---

## 2. Layer 2: Multi-Engine Portfolio with Instant Cancellation

No single formal algorithm wins on every circuit:

| Engine | Strength | Weakness |
|---|---|---|
| **Z3 BMC + k-Induction** | Shallow bugs, arithmetic / wide bitvectors | Deep sequential invariants needing strengthening |
| **ABC AIG IC3/PDR (`dc2; scorr; pdr`)** | Deep control FSMs, arbiters, pipelines | Bit-blasting wide multipliers (32x32+) |
| **ABC AIG Temporal Induction (`dc2; int`)** | Fast inductive proofs on simplified AIGs | Requires small k-inductive depth |
| **Word-Level BTOR2 (`pono` / `avr` / `btormc`)** | Wide datapaths, memories, 64-bit arithmetic | Requires external BTOR2 solver on `PATH` |

VeloFormal's **Portfolio Engine** (`--engine portfolio`, the default) races these engines concurrently across CPU cores using a shared lock-free `CancelToken`. The instant any worker proves the property or finds a counterexample, all sibling solver subprocesses are terminated cleanly within milliseconds.

---

## 3. Layer 3: Word-Level BTOR2 Hardware Model Checking

Bit-blasting a 64-bit datapath or a 256-entry register file into a bit-level AIG produces hundreds of thousands of AND gates. Word-level hardware model checkers — **Pono** (`ic3ia` / `ind`), **AVR**, and **BtorMC** — reason directly over fixed-width bitvectors and arrays using the standard **BTOR2** format.

VeloFormal compiles any SystemVerilog + `.vfc` design directly to validated BTOR2 (`crates/vf-engine/src/btor_engine.rs`) with zero manual translation:

```bash
# Auto-select the best installed word-level solver (pono -> avr -> btormc)
veloformal contract datapath.sv --spec datapath.vfc --engine btor

# Or explicitly select a specific word-level backend:
veloformal contract datapath.sv --spec datapath.vfc --engine pono
veloformal contract datapath.sv --spec datapath.vfc --engine avr
veloformal contract datapath.sv --spec datapath.vfc --engine btormc
```

### Automatic Portfolio Integration & Word-Level Trace Reconstruction

- Whenever `pono` or `avr` is installed on `PATH`, it **automatically joins Portfolio Worker 4** alongside Z3 and ABC — giving your proofs word-level IC3IA power with zero extra flags.
- When a BTOR2 solver refutes a property, VeloFormal automatically reconstructs a clean, human-readable **word-level counterexample trace**, **COI Root-Cause Backtrace**, and **interactive HTML waveform**.

---

## 4. Layer 4: Automated Hierarchical Assume-Guarantee Composition (`veloformal compose`)

Even with word-level IC3 and COI pruning, verifying an entire SoC flat in one monolithic solver run does not scale indefinitely. The mathematical solution used in industrial chip sign-off is **Assume-Guarantee Compositional Reasoning**:

$$\frac{\langle \text{Sub} \models R_{\text{sub}} \Rightarrow G_{\text{sub}} \rangle \quad \text{and} \quad \langle \text{Top}[\text{Sub} \mapsto (G_{\text{sub}} / R_{\text{sub}})] \models R_{\text{top}} \Rightarrow (R_{\text{sub}} \land G_{\text{top}}) \rangle}{\langle \text{Top} \parallel \text{Sub} \models R_{\text{top}} \Rightarrow G_{\text{top}} \rangle}$$

In plain English, `veloformal compose` automates this proof rule in **two sound phases**:

1. **Phase 1 — Leaf `Provider` Proofs**: Each submodule is compiled standalone and proved against its own `.vfc` contract in **Provider role** (`require` = assume, `invariant`/`ensure` = assert). If any leaf fails, composition aborts immediately.
2. **Phase 2 — Top-Level Blackboxing + `Caller` Binding**: VeloFormal elaborates the top-level SoC, **blackboxes** every verified submodule instance (deleting all its internal flip-flops and datapath logic while leaving its boundary ports unconstrained), automatically binds each submodule's `.vfc` contract onto its hierarchical instance prefix (`u_alu.in_a`, `u_alu.out_val`, ...) in **Caller role** (`require` = **assert** that the SoC drives legal inputs, `invariant`/`ensure` = **assume** the already-proved leaf guarantees), and proves the top-level SoC contract!

### Hands-On Example: `examples/24_hierarchical_compose/`

Consider a 2-stage pipelined leaf ALU (`heavy_alu_leaf.sv`, 7 internal flip-flops) instantiated inside `soc_top.sv`:

**Leaf Contract (`heavy_alu_leaf.vfc`)**:
```vfc
contract HeavyAluContract for heavy_alu_leaf {
    clock posedge clk;
    reset when rst;

    // Precondition: caller must clamp operands to [0..127] when in_valid is asserted.
    require bounded_inputs: !in_valid || (unsigned(in_a) <= 127 && unsigned(in_b) <= 127);

    // Guarantee: whenever out_valid is asserted, out_val fits in [0..254].
    invariant out_bounded: !out_valid || unsigned(out_val) <= 254;
}
```

**Top-Level SoC Contract (`soc_top.vfc`)**:
```vfc
contract SocTopContract for soc_top {
    clock posedge clk;
    reset when rst;

    invariant soc_output_bounded: !soc_valid || unsigned(soc_data) <= 254;
}
```

Run automated hierarchical composition with a single command:

```bash
veloformal compose \
  examples/24_hierarchical_compose/heavy_alu_leaf.sv \
  examples/24_hierarchical_compose/soc_top.sv \
  --top soc_top \
  --spec examples/24_hierarchical_compose/soc_top.vfc \
  --sub u_alu=examples/24_hierarchical_compose/heavy_alu_leaf.sv:heavy_alu_leaf:examples/24_hierarchical_compose/heavy_alu_leaf.vfc
```

Output:
```text
PROVED   [compose:leaf] `u_alu` (HeavyAluContract, 7 FFs) — portfolio(pdr)
PROVED   [compose:top]  `soc_top` (SocTopContract) — FFs 9 -> 2 (12 signals abstracted, winner: portfolio(pdr))
```

Notice the state-space collapse: **`FFs 9 -> 2`** — all 7 internal pipeline flip-flops of `u_alu` vanished from the top-level proof!

### Catching Soundness Bugs: Caller Precondition Violations

What if an engineer (or AI agent) edits `soc_top` and forgets to clamp `req_a` and `req_b` before driving `u_alu` (`soc_top_bad_caller.sv`)?

```bash
veloformal compose \
  examples/24_hierarchical_compose/heavy_alu_leaf.sv \
  examples/24_hierarchical_compose/soc_top_bad_caller.sv \
  --top soc_top_bad_caller \
  --spec examples/24_hierarchical_compose/soc_top.vfc \
  --sub u_alu=examples/24_hierarchical_compose/heavy_alu_leaf.sv:heavy_alu_leaf:examples/24_hierarchical_compose/heavy_alu_leaf.vfc \
  --report bad_caller.html
```

Because VeloFormal flips `u_alu`'s `require bounded_inputs` into an **assertion** in Phase 2 (`Caller` role), composition immediately **REFUTES** the buggy SoC and generates an interactive HTML waveform & COI backtrace pointing straight at the unclamped `req_a` / `req_b` bus inputs!

### Agentic Usage via `vf-mcp`

AI coding agents can invoke hierarchical composition directly via the `compose_verify` MCP tool, receiving structured JSON with per-leaf status, FF reduction counts, and a compact COI backtrace on failure.
