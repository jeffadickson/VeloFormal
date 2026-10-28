# VeloHdl structural envelope v1

`velohdl.structural.v1` is a deliberately narrow interchange boundary between
an HDL frontend and VeloFormal. It is not a SystemVerilog language definition,
and an accepted artifact is not itself a proof.

## Envelope

The root object contains exactly `format`, `provenance`, and `model`.
`format` is the literal `velohdl.structural.v1`. `provenance` contains exactly:

- `source_sha256` — lowercase SHA-256 of the decoded UTF-8 source text used by
  the producing command;
- `frontend` and `frontend_revision` — identity and selected revision of the
  HDL frontend;
- `normalized_ir_sha256` — lowercase SHA-256 of the decoded normalized JSON text;
- `adapter` and `adapter_version` — the lowering implementation and semantic
  subset version.

The consumer validates field shape and hash syntax. Translation from normalized
UHDM re-hashes the supplied normalized text. Re-reading an envelope alone does
not independently verify its claimed source or IR bytes: those bytes must be
provided to do that. Use `--verify-velohdl FILE --source FILE --normalized-ir
FILE` to perform that independent byte check.

## Accepted normalized inputs

The translator accepts the current `velosim.uhdm.normalized.v1` producer only
for its documented one-process reset/increment shape. It also accepts the
VeloHdl-owned, prospective `velohdl.normalized.v1` fixture grammar below. The
latter is not claimed to be emitted by the current VeloSim checkout.

The VeloHdl fixture grammar has:

- 1–256 uniquely named signals, each with a width from 1 through 256 and an
  optional Boolean `signed` flag (default `false`);
- 1–256 `always_ff` processes, all `posedge` on the same scalar clock and all
  using the same scalar active-low reset;
- one `if (!reset)` body per process, whose reset branch is a nonblocking write
  of zero and whose else branch is a nonblocking write to the same destination;
- one unique destination per process (multiple writers are rejected);
- else expressions formed from `const`, `ref`, `not`, `add`, `sub`, `mul`,
  `and`, `or`, `xor`, `shl`, `shr`, and `ite`;
- `ite` conditions formed from scalar `ref`, `not`, Boolean `and`/`or`, and
  `eq`, `ne`, `lt`, `le`, `gt`, or `ge`. Comparisons carry an explicit
  `width` from 1 through 256 and an optional Boolean `signed` flag (default
  `false`).

State destinations and non-scalar inputs are unsigned by default, or signed
bitvectors when their explicit `signed` flag is true. Unsigned scalar inputs are
Boolean controls. Constants inherit the destination signedness and must fit its
width. References of another bitvector width acquire an explicit `resize`;
scalar Boolean signals cannot silently become arithmetic operands. Signed and
unsigned arithmetic operands cannot mix implicitly. Comparisons remain an
explicit-width unsigned subset by default; `signed: true` selects signed
bitvector operands. Boolean controls cannot be used in signed comparisons.

## Formal mapping

At frame `t`, each destination's expression is evaluated over the complete
frame-`t` signal valuation. At `t + 1`, reset writes zero; otherwise the
expression becomes the next value. Consequently, distinct processes have
nonblocking, concurrent semantics even when one expression reads another
process's state. Initial state is deliberately unconstrained: a reset branch is
not treated as evidence that reset occurred before the checked trace.

The resulting model is passed to VeloFormal's bounded Z3 checker. `bounded-pass`
only states that no violation was found through the requested finite depth; it
is not an induction or PDR proof.

## Rejected semantics

The v1 bridge rejects multiple clocks, distinct resets, clock/reset destinations, latches, blocking
assignments, multiple writers, non-zero reset assignments, arrays/memories,
four-state values, hierarchy, parameters, delays, asynchronous resets, and
any unlisted expression node. These are requirements for a later envelope
version, not implicit behavior.
