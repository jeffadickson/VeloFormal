# VeloSim RTL bridge work package

Status: **planned; blocked on a clean VeloSim source snapshot.** This package
does not merge the VeloSim and VeloFormal projects and does not treat simulator
execution as a formal proof.

## Goal

Allow VeloFormal to consume a limited, versioned structural RTL representation
produced by the separately built VeloSim project. The existing explicit
transition-model JSON remains the trusted direct input and differential oracle
until the bridge has qualified the same supported subset.

## Import boundary

When VeloSim's mutation work reaches a selected committed revision, import it
under `engines/VeloSim/` as an independent project. Do not copy its dirty work
tree, generated artifacts, build output, or third-party Surelog/UHDM trees.
Record the upstream revision and dependency/tool versions in the import manifest.

## Deliverables

1. A versioned bridge input based on VeloSim's normalized structural RTL IR.
2. An adapter that accepts only one-clock, explicitly bounded, supported RTL and
   emits VeloFormal's transition model and carries declared source/frontend/IR/adapter provenance into solver evidence.
3. Differential fixtures that compare VeloSim simulation traces, the generated
   transition model, and VeloFormal bounded-checking outcomes.
4. Explicit rejection diagnostics for four-state behavior, unsupported scheduling,
   unconstrained hierarchy, or constructs whose formal meaning is not defined.

## Implemented first slice

`VeloSimNormalizedTranslator` currently accepts the current VeloSim normalized
UHDM shape for
one through 256 `always_ff` processes on one shared `posedge` clock with one
shared active-low reset branch
that assigns zero and a non-reset branch that contains a typed expression. The
accepted expression nodes are `const`, `ref`, `not`, `add`, `sub`, `and`, `or`,
`xor`, and `ite`; each arithmetic value is explicitly destination-width typed.
It emits one unsigned 1--256-bit VeloFormal state per process destination, with all other
one-bit signals represented as Boolean inputs and wider signals as unsigned
inputs. Different process clocks/resets, edges, duplicate writers, branch targets,
out-of-range constants, four-state data, and unsupported expressions fail closed.
This is an implemented starter subset, not general RTL lowering. The expanded
`velohdl.normalized.v1` fixture grammar is intentionally separate from the
current VeloSim `velosim.uhdm.normalized.v1` producer; it is a prospective
integration contract, not evidence that the current VeloSim checkout emits it.
The translator verifies the supplied normalized-IR hash at translation time. A
consumed envelope carries declared source and IR hashes; it cannot independently
re-hash those artifacts unless their exact bytes are supplied alongside it.

## DDR-controller readiness boundary

The implemented subset is suitable only for validating the bridge and simple
single-clock control examples. It is not yet suitable for finding bugs in a DDR
memory controller. That requires subsequent qualified support for multiple
synchronous processes, controller memories and bounded queues, an abstract DRAM
transaction model, command/timing properties, and separately modelled clock-domain
crossings. Any future claim of DDR bug-finding capability must identify which of
those capabilities and controller configuration were actually exercised.

Use the bridge CLI to produce an inspectable envelope before checking a contract:

```powershell
dotnet run --project src/VeloFormal.Cli -- --translate-velosim-normalized counter.normalized.json --module counter --source counter.sv --frontend-revision REVISION > counter.velohdl.json
dotnet run --project src/VeloFormal.Cli -- --verify-velohdl counter.velohdl.json --source counter.sv --normalized-ir counter.normalized.json
dotnet run --project src/VeloFormal.Cli -- --check-contract counter.vfc --model counter.velohdl.json --depth 16
```

The redirect is intentional: the first command writes only the versioned JSON
envelope to standard output, while input errors use standard error and exit 2.
The second command verifies claimed source and normalized-IR hashes; the check
command carries provenance into its solver JSON but does not repeat byte checks.

## Future acceptance gates (not yet achieved)

- A pinned VeloSim revision builds independently from the VeloFormal solution.
- Every bridge fixture has recorded source, frontend, IR-schema and adapter identity/version, and the translation path verifies the normalized-IR hash.
- Positive fixtures agree with independent trace behavior; malformed and unsupported
  RTL cases fail closed with source-located diagnostics where the frontend provides locations.
- A VeloFormal result is still labelled `bounded-pass`, `counterexample`,
  `incomplete`, or `unknown` according to its own solver evidence; no VeloSim run
  is promoted to a proof.

## Issue handoff

Confirmed bridge defects, unsupported constructs, semantic discrepancies, and
provenance gaps are tracked as GitHub Issues before they are deferred. Each issue
records the VeloHdl envelope/IR version, available VeloSim revision, minimized
fixture or reproducer, expected and observed behavior, and whether the finding is
simulation-only, formal-only, or cross-tool. This makes the work transferable to
VeloSim without treating an unverified suspicion as a defect.
