# VeloHdl Yosys-compatibility program

## Intent

VeloFormal aims to become capable of formally checking the useful RTL classes
that a Yosys/SymbiYosys flow can check. This does **not** mean claiming full
IEEE SystemVerilog compatibility or reimplementing every Yosys backend. Each
capability enters the supported profile only after an independently runnable,
version-pinned comparison against Yosys/SymbiYosys.

## Current baseline

The current VeloHdl bridge accepts the actual current VeloSim normalized-v1
counter shape and a separately versioned VeloHdl fixture grammar for one-clock,
active-low-reset, nonblocking state updates. The fixture grammar now includes
typed arithmetic, muxes, and explicit-width comparisons. The local machine did
not have `yosys`, `yosys-smtbmc`, `sby`, `iverilog`, `verilator`, or `slang` on
2026-10-28, so no Yosys differential outcome is claimed.

## Compatibility ladder

| Stage | Capability | Qualification evidence |
|---|---|---|
| 0 | Versioned structural artifact, provenance, BMC, counterexamples | Implemented locally; see bridge verification record. |
| 1 | Single-clock register-transfer subset: arithmetic, muxes, comparisons, enables, reset | Paired VeloHdl/Yosys fixtures with matching bounded pass/fail traces. |
| 2 | Assertions, assumptions, covers, symbolic inputs, `$past` family | Property-level status and witness comparison with `yosys-smtbmc`. |
| 3 | Parameters, hierarchy, memories/arrays, and synchronous memory ports | Differential results under declared abstraction/bit-blasting choices. |
| 4 | Induction/PDR and formal proof provenance | Independent unbounded result agreement; never inferred from BMC. |
| 5 | Multi-clock and liveness | Explicit clock scheduling and engine-specific compatibility evidence. |

## Differential contract

For a fixture in a qualified stage, record:

- exact HDL source and expected frontend/elaboration settings;
- VeloHdl envelope, source/IR hashes, adapter version, and contract;
- Yosys/SymbiYosys, solver, frontend, and script versions;
- finite depth or proof mode, assumptions, result status, and trace/witness;
- any semantic normalization such as two-state treatment or memory abstraction.

A pass/pass comparison is not sufficient when the reachable state spaces differ.
Counterexamples must replay in both models or the discrepancy becomes a tracked
GitHub issue with a minimized reproducer. No unsupported construct is silently
approximated.

## Near-term next action

Install or obtain a pinned Yosys/SymbiYosys environment, then qualify Stage 1
using small register/mux/comparator fixtures before enabling any claim of
Yosys-compatible behavior.
