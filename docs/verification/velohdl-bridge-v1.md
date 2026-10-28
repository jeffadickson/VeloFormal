# VeloHdl bridge v1 verification record

## Scope

This record covers the uncommitted VeloHdl bridge work based on Git baseline
`b31d8fba96ba0cce79580a74a5105ecbba1b236b` (`Checkpoint 650: add verification
tooling`). It is evidence for the documented v1 subset, not a claim of general
SystemVerilog, VeloSim, or DDR-controller conformance.

## Local execution

On 2026-10-27 (America/Los_Angeles), using .NET SDK 8.0.133, the following
commands completed successfully from the repository root:

```powershell
dotnet build VeloFormal.sln -c Release --no-restore
dotnet test tests/VeloFormal.Tests/VeloFormal.Tests.csproj -c Release --no-build --logger 'console;verbosity=minimal'
dotnet run --project tools/VeloFormal.Build -c Release -- coverage
```

The final ordinary regression and coverage-helper executions each reported 648
passed, 0 failed, and 0 skipped tests. The latter generated
`artifacts/coverage/20261028-030727-0647689/` with Coverlet and ReportGenerator
5.4.3. Its scope includes VeloFormal, VeloHdl, and VeloFormal.Cli: 96.7% line
coverage (1448/1497), 83.8% branch coverage (1712/2042), and 99.2% method
coverage (260/262). These totals are not comparable with the older two-assembly
measurements because the CLI assembly was added to scope.

Focused bridge execution reported 12 passed, 0 failed, and 0 skipped tests. It
covers versioned-envelope parsing, provenance propagation into solver JSON,
normalized-IR hash mismatch, malformed fields, width preservation, supported
expressions, unsupported operators, overflow rejection, concurrent multi-state
lowering, duplicate-writer rejection, checked-in fixture copying, and CLI
translation followed by CLI bounded checking.

## Simulated peer review

A simulated peer review of the local bridge delta found that clock/reset could
be written as state, the expanded grammar was incorrectly attributed to the
current VeloSim v1 producer, provenance wording implied raw-byte hashing, and
the work-package acceptance list blurred present evidence with future gates.
All four findings were accepted: clock/reset destinations are now rejected;
expanded fixtures use `velohdl.normalized.v1`; documentation specifies decoded
UTF-8 text hashing; and external VeloSim build/trace/source-location items are
labelled future gates. The review also noted stale single-register wording,
which was corrected. This was a simulated review, not an external expert audit.

## Limits

- No Stryker mutation score is claimed: the pinned 4.16.0 tool remains blocked
  at project discovery for this net8.0 solution.
- StrictDoc export is not claimed: `strictdoc` was unavailable on this machine.
- The external VeloSim checkout was not imported or built here because its
  mutation work tree was dirty. The checked-in normalized fixtures are the
  reproducible integration boundary.
- These runs establish finite bounded-checking behavior only. They do not prove
  an unbounded invariant, elaboration equivalence, or any DDR-controller claim.
