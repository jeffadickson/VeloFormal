# VeloFormal project rules

## VeloHdl and VeloSim coordination

- Record every confirmed VeloHdl integration defect, unsupported construct, semantic discrepancy, or frontend/provenance gap as a GitHub Issue before treating it as deferred work.
- Each issue must identify the VeloHdl envelope/IR version, VeloSim revision (when available), reproducer inputs or fixture, expected versus observed behavior, and whether the finding is simulation-only, formal-only, or cross-tool.
- Link the issue from the affected VeloFormal requirement, test, or bridge documentation so it can be carried to VeloSim without relying on chat history.
- Do not file an issue merely for an unverified suspicion; retain a local investigation note until a minimal reproducer or clear evidence exists.
