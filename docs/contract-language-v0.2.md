# VeloFormal Contract Language — v0.2 design candidate

Status: **proposed specification**, not implemented. Scope: RTL-oriented assume/guarantee contracts, not a new general-purpose programming language. Existing README describes a restricted SystemVerilog/Z3 prototype; this specification does not imply full SV/SVA support.

## Twenty iterative design passes

| Pass | Issue | Decision | Acceptance condition |
|---|---|---|---|
| 01 | Ambiguous contract target | `contract Name for module_name`; hierarchical bindings explicit | Unknown port/module rejected |
| 02 | Eiffel responsibilities | `require` is environment assumption for provider verification; `ensure` provider guarantee | Check and apply reverse proof obligations |
| 03 | Ambiguous `invariant` | `invariant` is provider guarantee at each active sampled tick | Invariant violations yield traces |
| 04 | Clock domain | Mandatory `clock posedge clk` in v0.2 | Unclocked temporal clauses rejected |
| 05 | Reset meaning | `reset when !rst_n`; reset samples disable obligations; pending obligations cancelled | Reset-inflight semantics tested |
| 06 | Values vs cycles | `prev(expr)` is previous active sample; only well-defined after history is available | At first sample prev() guard required |
| 07 | Eiffel `old` ambiguity | `at_trigger(expr)` captures value at operation acceptance; not prior tick | Old-value semantics tested on changing count |
| 08 | Operational handshake | `operation write when valid && ready` defines acceptance | Stalled valid alone creates no obligation |
| 09 | Temporal guarantee | `ensure after N cycles: expr` checks at t+N | Off-by-one test |
| 10 | Latency window | `ensure within A..B cycles: expr` requires one satisfaction in inclusive window | Late-only responses fail |
| 11 | Safety during latency | `ensure until response: expr` deferred; reject until formal event semantics designed | Unsupported syntax diagnostic |
| 12 | Multiple transactions | Every accepted event spawns an independent obligation | Two overlapping writes are tracked |
| 13 | Resource bounds | Finite windows required for liveness in v0.2 | Unbounded eventual rejected |
| 14 | Expression typing | Explicit widths, signedness; no silent widening or signed/unsigned coercion | Width mismatch diagnostic |
| 15 | Four-state HDL | v0.2 is explicitly two-state; X/Z semantics excluded, not silently modeled | Four-state request rejected |
| 16 | Environment satisfiability | Report vacuity, unreachable trigger, and inconsistent assumptions separately | Unsatisfiable require set reported |
| 17 | Composition | Verify provider requires => ensures, then caller discharges requirements | Assuming unproved guarantee forbidden |
| 18 | Parameterization | Elaborate module parameters before proof; static bounds validated | Nonpositive depth rejected |
| 19 | Diagnostics | Each clause has stable label and clock/sample index in traces | Counterexample maps to source clause |
| 20 | Proof reporting | Distinguish proved / counterexample / bounded-pass / unknown / unsupported | BMC success never reported as proof |

## Language sketch (not yet parser grammar)

```text
contract FifoSafety for fifo
    parameter DEPTH: integer where DEPTH > 0
    clock posedge clk
    reset when !rst_n

    require environment_input_valid:
        !(rd_en && empty && !wr_en)

    invariant count_in_range:
        unsigned(count) <= DEPTH

    invariant flags_consistent:
        empty == (count == 0)

    operation write when wr_en && !full && !rd_en:
        ensure occupancy_rises after 1 cycles:
            count == at_trigger(count) + 1

    cover becomes_full:
        full
end
```

### Formal definitions

Let `S_t` be design state sampled at active clock tick `t`, `I_t` inputs, and `T(S_t,I_t,S_(t+1))` the transition relation. Reset handling is explicit: active monitoring begins with the first non-reset sample, and active pending obligations are cancelled at reset assertion.

- **Provider verification**: under `Init ∧ T ∧ Require`, prove `Invariant` for each active reachable sample and every activated `Ensure` obligation. Assumptions are not automatically asserted as design guarantees.
- **Caller verification**: prove input `Require` at the boundary, and only after provider verification succeeds may the caller assume the provider's `Ensure`. An unverified or merely bounded guarantee MUST NOT be promoted to an assumption.
- **Trigger sampling**: if operation `when P` is true at `t`, capture `at_trigger(e) = e(S_t,I_t)`, and create a separate obligation instance.
- **Exact-delay**: `ensure after N cycles: Q` requires `Q` true at `t+N`; `N >= 1`.
- **Bounded eventuality**: `ensure within A..B cycles: Q` requires `Q` true on at least one sample in inclusive `[t+A,t+B]`; `1 <= A <= B`.
- **Invariant**: `G(active => Inv)`. Initial reset proof and inductive reachability obligations are different from runtime observation.
- **Vacuity**: if a guarantee's trigger is unreachable given environmental assumptions, mark as vacuous, not meaningful evidence of successful operation. A separate cover goal can establish reachability.
- **History**: `prev` is undefined at the first active sample. Require an explicit `history_valid` guard or reject such use.
- **Type model**: `logic[n]` maps to n-bit bitvectors; signed arithmetic must be explicit and fixed-width; mathematical `integer` allowed for static parameters only in v0.2. Overflow of bitvectors wraps unless explicit widening is requested.

### Lowering strategy

1. Resolve module and parameter bindings, width/type-check expressions against an elaborated RTL symbol table.
2. Produce typed clause IR: label, source span, clock/reset, role, trigger, delay interval, sampled captures, expression AST.
3. Compile immediate invariants and requires into clocked assertions/assumptions. Compile delayed ensures into obligation monitors (including independent concurrent instances).
4. Backends: emit SVA for simulator/formal-tool interop; emit transition-system constraints to Z3/BTOR2 for BMC. PDR or induction can be added without changing the source language.
5. Record proof status and assumption set per property; require certified/provider-verified contracts before compositional abstraction.

### Deliberately unsupported in v0.2

- Cross-clock-domain temporal contracts, analog time, async reset transitions between sampling edges, full 4-state semantics, unbounded liveness/fairness, unconstrained recursion, and general SVA sequence operators.
- Implicit proof of data ordering in a FIFO from a count-only specification. Separate data-integrity contracts are required.
- Treating a bounded-pass as an unbounded proof.

## Conformance cases

| ID | Fixture | Expected outcome |
|---|---|---|
| C01 | Parameter DEPTH=0 | Elaboration error |
| C02 | `after 0 cycles` | Static error |
| C03 | `within 5..3 cycles` | Static error |
| C04 | Unknown `count` symbol | Binding error |
| C05 | `prev(count)` without history guard | Static error |
| C06 | Write accepted at t=2, count increments at t=3 | Pass exact-delay |
| C07 | Write accepted at t=2, count increments at t=4 | Counterexample at t=3 |
| C08 | Two accepted writes at t=2,3 | Two independent obligations |
| C09 | Reset asserted between trigger and deadline | Obligation cancelled (documented policy) |
| C10 | Response exactly at upper window bound | Pass |
| C11 | Response one sample after upper bound | Counterexample |
| C12 | Inconsistent requirements | Assumption-unsat/vacuous diagnostic |
| C13 | No valid write ever occurs | Trigger-vacuous diagnostic |
| C14 | BMC depth 100 finds no violation | Bounded-pass, not proved |
| C15 | One unverified child contract used as assumption | Composition error |
| C16 | Cross-clock reference without synchronization model | Unsupported error |
| C17 | Implicit signed/unsigned comparison | Type error |
| C18 | User requests four-state X-sensitive proof | Unsupported error |
| C19 | Reachable invariant breach | Counterexample with clause label |
| C20 | Fully discharged inductive proof | Proved with engine/model metadata |

## Open decisions requiring implementation experiments

- Whether the syntax embeds expressions as a strict SV subset or uses a dedicated bitvector expression grammar.
- Whether monitor generation supports arbitrarily many overlapping requests via symbolic obligations or requires bounded outstanding depth.
- How to store provider-proof provenance (design hash, elaboration parameters, assumptions, tool version, solver version) to prevent unsound reuse.
- How to express legal FIFO simultaneous push/pop semantics and multi-clock CDC without overspecifying implementations.

References: CIRCT Verif dialect contracts https://circt.llvm.org/docs/Dialects/Verif/ ; SVA IEEE 1800 conceptual model; SymbiYosys/Yosys formal workflows.
