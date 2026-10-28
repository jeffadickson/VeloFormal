# VeloFormal

For VCD/JSONL commands, API limits, and replay/export semantics, see [recorded trace workflows](docs/trace-workflows.md).

Active development now targets C#/.NET 8. The Python package below is retained
as a compatibility reference. Run the C# checker with:

```console
dotnet run --project src/VeloFormal.Cli -- --trace examples/occupancy.trace
dotnet run --project tools/VeloFormal.Build -c Release -- coverage
```

The C# helper builds the solution, runs xUnit with Coverlet, restores the pinned
ReportGenerator tool, and writes HTML, text, and Cobertura reports to a fresh
directory under `artifacts/coverage/`. Use `test` instead of `coverage` to omit
coverage. All development helpers are C#; no Python helper is required.

## Development verification

The repository keeps the core checker on Microsoft.Z3 4.12.2: this is the
newest published `Microsoft.Z3` package as of 2026-10-27, so no speculative
solver upgrade has been applied. The release test suite exercises its native
Z3 integration on this machine.

`FsCheck.Xunit` supplies deterministic formatter-idempotence properties; the
current replay seed is `123456,789013`. `System.CommandLine` validates the CLI
option vocabulary before the compatibility-preserving command adapter dispatches
it. StrictDoc requirements reside in `requirements/VeloFormal.sdoc` and can be
exported with `strictdoc export requirements --output-dir artifacts/strictdoc`.

`fuzz/VeloFormal.Fuzz` is a parser-only SharpFuzz target. Run actual campaigns
from Linux/WSL using AFL++ or libFuzzer, recording corpus, seed, time, and
minimized crashes; ordinary unit tests are not a substitute. The local tool
manifest also pins Stryker.NET 4.16.0. Its project-discovery run is currently
blocked on this net8.0 solution before mutation generation; this is a tool
compatibility limitation, not a mutation score. The checked-in config lives
beside the xUnit project for the documented invocation model.

To simulate the pinned ISC-licensed PicoRV32 UART divider fixture, install
Icarus Verilog and run:

```console
dotnet run --project tools/VeloFormal.Build -c Release -- rtl
```

The helper uses `iverilog` and `vvp` from PATH. Set `IVERILOG` and `VVP` for
explicit executable paths, and `IVL_ROOT` when the compiler needs its library
directory. Waveforms stay under `artifacts/rtl/`. The restricted C# VCD reader
samples the settled values at each known 0-to-1 clock edge. Selected signals
must have known two-state values; X/Z is rejected. Values are unsigned, and
unsupported VCD constructs, clock glitches within one timestamp, unknown
signals, and decreasing timestamps fail explicitly. This sampling policy is
for simulation observation and is distinct from SVA preponed-region sampling.
See [fixture provenance](tests/fixtures/picorv32/README.md).

The C# port supports both immutable trace checking and a bounded Z3 checker for the v0.2 contract grammar. The formal path accepts an explicit two-state transition model, not arbitrary Verilog elaboration:

```console
dotnet run --project src/VeloFormal.Cli -- --parse-contract examples/counter.vfc
dotnet run --project src/VeloFormal.Cli -- --check-contract examples/counter.vfc --model examples/counter.model --depth 3
```

The checked-in counter example intentionally reports `incomplete` at finite depth because an unconstrained final-sample increment can still create an obligation past the horizon. A `bounded-pass` is finite-horizon evidence only, never a proof. See [the contract-language profile](docs/contract-language-v0.2.md) for supported syntax, status codes, and exclusions.

## Python compatibility reference

The earlier Python API and tests are retained below for comparison during the
C# migration. Python is not required by the active C# checker or build helpers.

The published repository now includes an executable Python contract trace checker, based on [the v0.2 contract design](docs/contract-language-v0.2.md). It checks labeled requirements, invariants, exact-delay and inclusive bounded-window responses, independent overlapping obligations, sampled reset cancellation, and cover observations.

**Status:** Research prototype. Not full IEEE 1800 or a drop-in replacement for Yosys/SymbiYosys. Proof results are valid only under the supported two-state model and declared assumptions.

The earlier v1.2 SystemVerilog/Z3 implementation described in the development conversation is absent from the published history. The current C# profile supplies bounded model checking only for explicit transition models; induction, arbitrary RTL parsing, and solver-backed vacuity proof are not available. The existing package version is retained for continuity; it does not certify those capabilities.

Run with Python 3.10 or later (the trace API uses only the standard library):

```console
python -m unittest discover -s tests -v
python -m veloformal --demo
python -m veloformal --trace examples/occupancy.trace
```

For the installed `veloformal` command, install with `python -m pip install -e .`.

```python
from veloformal import Contract, Ensure, check_trace

contract = Contract(ensures=(Ensure(
    "occupancy_rises",
    trigger=lambda s: s["write"],
    response=lambda s, captured: s["count"] == captured["count"] + 1,
),))
report = check_trace(contract, [
    {"write": True, "count": 0},
    {"write": False, "count": 1},
])
assert report.status == "trace-pass"
```

Each input mapping represents one clock edge. `Ensure` defaults to a one-cycle delay; set `earliest` and `latest` for a finite inclusive window. The response receives a snapshot of the triggering sample. Reset samples disable checks and cancel pending obligations. Predicates must be pure and return booleans; missing signals and predicate exceptions propagate to the caller. Python arithmetic does not implement RTL width or signedness rules.

Results are `trace-pass`, `counterexample`, `incomplete` (unfinished obligations or no active samples), or `invalid-assumptions`. A failed requirement invalidates the trace as provider evidence and stops checking. Findings include zero-based sample indices and trigger indices for delayed failures. `untriggered` records triggers absent from this trace; it does not establish formal unreachability. Covers record observed hits only. **A trace pass is never a bounded or unbounded formal proof.**

Next steps include proof provenance, induction/PDR, and RTL elaboration. `prev`, compositional proof, and arbitrary SystemVerilog parsing remain intentionally unsupported in the current explicit-transition bounded-checking profile.

### Recorded trace input

`--trace FILE` reads a JSON document containing `contract` and `samples`; `--trace -` reads stdin. See [the occupancy example](examples/occupancy.trace). The contract accepts `reset`, label-to-expression maps for `requires`, `invariants`, and `covers`, and an `ensures` array. Each ensure has `label`, `when`, `response`, and exactly one of `after: N` or `within: [A, B]`.

Expressions support signal names, integer/boolean literals (`True` and `False`), `and`, `or`, `not`, comparisons, parentheses, addition, subtraction, and bitwise `&`, `|`, `^`, `~`. Responses may use `at_trigger(expression)`. Sample values must be JSON booleans or integers. Arithmetic uses Python integers; this format has no implicit fixed-width wrapping or RTL elaboration. Attribute access, indexing, other calls, and nested captures are rejected. Unknown fields and invalid delay bounds are errors. Missing signals are errors when their expression is evaluated; reset-disabled samples may omit inactive signals.

The CLI prints a JSON report. Exit code 0 means `trace-pass`, 1 means any other observation status, and 2 means invalid input or a file error. No CLI result constitutes formal proof.
