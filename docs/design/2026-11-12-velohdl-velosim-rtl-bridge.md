# VeloHdl / VeloSim RTL bridge — design

**Date:** 2026-11-12  
**Decision:** consume VeloSim structural RTL as a separately versioned producer; do not merge the VeloSim runtime or duplicate its SystemVerilog frontend in VeloFormal.

## Goal

Make a first useful formal input path from a VeloSim-produced
`velosim.rtl.v1` document into VeloFormal's existing
`velohdl.structural.v1` envelope and bounded Z3 checker. Every accepted
artifact must identify its exact producer revision, source bytes, IR bytes,
adapter version, and formal subset.

This advances `VF-RTL-001`. It does not by itself qualify a Yosys-compatible
subset under `VF-RTL-002`; that needs paired VeloSim/Yosys evidence.

## Reuse-before-build assessment

| Candidate | Decision | Evidence and reason |
| --- | --- | --- |
| VeloSim `velosim.rtl.v1` producer | Reuse | Its managed `RtlDesign` validates the current versioned artifact and its source tree already emits rich structural RTL. Reusing it keeps UHDM/Surelog elaboration in its owning project. |
| Existing VeloFormal normalized-UHDM translator | Retain as a legacy fixture path | It accepts only the older counter-shaped `velosim.uhdm.normalized.v1`, so it cannot faithfully consume the current structural producer. |
| A VeloFormal SystemVerilog/UHDM parser | Reject | It would duplicate VeloSim frontend scope and break the intended ownership/provenance boundary. |
| VeloSim Z3 constraint randomization | Do not reuse as the formal engine | It provides simulation-side constraint randomization, not VeloFormal's transition relation, property contract, or proof status semantics. |
| New parser library | Do not add in this slice | `System.Text.Json` is sufficient for a bounded, closed JSON envelope; the domain work is semantic validation and lowering, not general JSON parsing. |

## Chosen boundary

```text
SystemVerilog -> VeloSim frontend -> velosim.rtl.v1 + producer manifest
                                      |
                                      v
                       VeloHdl structural RTL bridge (this work)
                                      |
                                      v
                  velohdl.structural.v1 + verification provenance
                                      |
                                      v
                       VeloFormal bounded Z3 checker
```

The bridge is a consumer, not a copy of VeloSim. The checked-in fixture uses
the producer schema and carries a pinned VeloSim commit. A future producer
update creates a new manifest and fixture, runs compatibility tests, and is
accepted only after the supported-subset result is recorded.

The first direct-export fixture was generated from VeloSim revision
`3a2953c4faaafab846a9b8ffd03b0935660b9054`. Its legacy `compile` path failed
after UHDM elaboration because it routed the structural output through the old
normalized-UHDM consumer; the direct exporter succeeds. This confirmed
formal-only integration gap is tracked as
[VeloFormal issue #2](https://github.com/jeffadickson/VeloFormal/issues/2).

## Accepted Stage A subset

The adapter accepts exactly one synchronous clocked design:

- `format` is `velosim.rtl.v1`;
- one scalar `input` clock, named by `clock`;
- no `timed`, `tasks`, assertions, named events, or clocking metadata;
- only two-state signals and constants; no `x` or `z` bits;
- scalar and packed signals of width 1 through 256, with explicit signedness;
- no memories, no arrays, and no dynamic containers;
- exact empty `initialization`, `continuous`, and `combinational` forms are
  accepted because VeloSim's native exporter emits them even when no work is
  present; nonempty forms are rejected in Stage A;
- one root `body`, either VeloSim's normal implicit-clock statement body, a
  single explicit edge process, or a root parallel set of processes. Explicit
  processes must all be `posedge` of the declared clock. The implicit form is
  interpreted as a single `posedge clock` process, matching the producer's
  normal one-process export;
- each formal state word has one writer; a process may use blocking local
  temporaries only after their precise semantics are specified in a later
  increment; Stage A initially accepts nonblocking assignments to whole signal
  references;
- expressions initially cover references, binary constants, `castsigned`,
  `castunsigned`, arithmetic/bitwise binary operators, comparisons, unary
  operators, and conditional expressions only when their widths and signedness
  can be lowered without implicit conversion.

Reset and initialization are not inferred. A reset branch is ordinary logic
until the adapter has an explicit reset contract. `initialization` is rejected
in the first increment rather than becoming a guessed time-zero state.

## Semantic mapping

Each accepted process is evaluated over frame *t*. All nonblocking updates
commit together into frame *t + 1*. Inputs are unconstrained at each frame
unless constrained by the VeloFormal contract. State is unconstrained at frame
zero unless a later, explicitly qualified initialization feature supplies a
complete initial-state contract. This preserves the checker’s bounded-result
semantics: a bounded pass is not an unbounded proof.

The bridge must reject rather than approximate four-state logic, delta cycles,
timed execution, multiple clocks, asynchronous events, memory behavior,
multiple writers, unsupported selections, or ambiguous SystemVerilog sizing.

## Provenance and update tracking

The new producer manifest is a checked-in JSON document beside each fixture.
It records:

- VeloSim repository URL, immutable commit, and dirty-tree status;
- producer artifact format and SHA-256 over its decoded UTF-8 text;
- source file SHA-256 over its decoded UTF-8 text and source-relative path;
- producer command and tool versions when available;
- VeloHdl adapter semantic-subset identifier and revision;
- expected VeloFormal result and the paired external-tool status, if qualified.

The VeloHdl envelope preserves the text hashes plus frontend and adapter
identities. The CLI will verify decoded UTF-8 text, matching
`StructuralProvenance.CalculateSha256`; it must not claim raw-byte verification
until a separately versioned raw-byte hash field is added. A changed producer
revision is never silently substituted for a pinned fixture.

## .NET 11 migration

Before landing the bridge implementation, update the root SDK pin and shared
target framework from .NET 8 to .NET 11 after a clean restore/build proves the
existing dependency set is compatible. Keep nullable reference types and
warnings-as-errors. Use C# 14 features selectively: primary constructors or
records for immutable manifest/model types, collection expressions for fixed
sets, and pattern matching for closed IR discriminators. Do not make syntax
changes that obscure the semantic lowering.

## Vertical slices

1. Migrate the solution to .NET 11 and establish a pinned structural fixture
   plus manifest from VeloSim revision `3a2953c`.
2. Add a strict `velosim.rtl.v1` parser and rejection tests for out-of-subset
   producer constructs.
3. Lower the smallest synchronous NBA counter into `StructuralDesign`; test
   provenance verification and bounded counterexample replay.
4. Add concurrent-NBA and signed-operation fixtures, then compare declared
   outcomes with pinned Yosys/SymbiYosys runs before extending the qualified
   subset.

## Validation

Each increment adds malformed-input and negative tests. The bridge parser gets
FsCheck properties for rejection of unknown nodes/fields and deterministic
translation. Coverage will use Coverlet/ReportGenerator. Mutation testing will
use Stryker.NET if its existing project-discovery defect is resolved; otherwise
the limitation remains recorded rather than replaced by a claimed score.

Confirmed producer/frontend mismatch or unsupported construct discovered while
executing these slices must be filed as a VeloHdl GitHub issue with the
envelope/IR version, VeloSim revision, minimal fixture, expected/observed
behavior, and simulation/formal/cross-tool classification. The relevant
requirement and bridge evidence must link to that issue.

## Out of scope

This decision does not merge repositories, claim full SystemVerilog,
SymbiYosys, Yosys, or DDR-controller compatibility, or treat VeloSim's Z3
constraint randomizer as a proof engine. Ordinary VeloSim `logic` declarations
remain out of Stage A because the producer marks them four-state; the first
fixture must use explicit two-state (`bit`) declarations or an otherwise
qualified two-state producer form.
