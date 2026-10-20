# Development checkpoints

The requested 200 coding increments are tracked by individually numbered regression
cases, in four batches of 50. Each batch adds implementation and regression code;
builds and tests run at its boundary, followed by one local commit. This counts
regression-driven increments, not 200 separate full feature rewrites or test runs.

| Range | Scope | Verification |
|---|---|---|
| 001–050 | C# solution, immutable samples, monitor port, expression parser, CLI, coverage helper; 50 expression cases | Release build: zero warnings/errors; 50/50 passed, Coverlet XML generated |
| 051–100 | Temporal/reset/assumption regressions and clause/sample input diagnostics | Release build: zero warnings/errors; 100/100 cumulative tests passed; Coverlet and HTML/Text/Cobertura reports generated |
| 101–150 | Input validation and CLI regressions | Release build: zero warnings/errors; 150/150 cumulative tests passed; Coverlet and HTML/Text/Cobertura reports generated |
| 151–200 | Restricted VCD reader, pinned PicoRV32 divider fixture, C# simulator helper, robustness/integration regressions | Release build: zero warnings/errors; 200/200 cumulative tests passed; 97.3% line / 90.5% branch coverage; RTL trace-pass on 53 active samples |

## Final verification

- The retained Python reference suite also passed: 23/23 tests.
- Coverlet measures the `VeloFormal` and `VeloFormal.Cli` assemblies; the build
  helper executable and Python reference are outside those percentages.
- ReportGenerator 5.4.3 produced HTML, text, and Cobertura reports under
  `artifacts/coverage/20261020-175851-9027629/` in the local checkout.
- The real Icarus run exercised 16 byte-write masks with three data patterns,
  hold cycles, and reset. It exposed and resolved Windows compiler DLL search
  and VCD parameter/dumpall compatibility issues. Waveform evidence is under
  `artifacts/rtl/37f00ba20ea5495db8c44d68eceb7f3f/` locally.
- These are finite trace observations. RTL elaboration and solver-backed
  formal verification remain outside the implemented scope.

The original uncommitted Python prototype is retained as a compatibility reference.
No prior solver implementation was present in the published Git history.
