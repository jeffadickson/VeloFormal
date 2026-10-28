# VeloHdl Stage-1 pre-differential verification

## Scope

This record covers the local Stage-1 expansion after commit `35dd442`: explicit
comparison conditions, multiplication, and logical shifts in the prospective
`velohdl.normalized.v1` fixture grammar. It is **not** differential evidence
against Yosys or SymbiYosys.

## Execution

On 2026-10-28 (America/Los_Angeles), .NET SDK 8.0.133 ran:

```powershell
dotnet build VeloFormal.sln -c Release --no-restore
dotnet test tests/VeloFormal.Tests/VeloFormal.Tests.csproj -c Release --no-build --filter 'FullyQualifiedName~VeloHdlAdapterTests'
dotnet run --project tools/VeloFormal.Build -c Release -- coverage
```

The final focused bridge suite had 18 passed, 0 failed, and 0 skipped tests.
The final full Coverlet/ReportGenerator run had 654 passed, 0 failed, and 0
skipped tests.
Coverage scope included VeloFormal, VeloFormal.Cli, and VeloHdl: line 96.3%
(1494/1551), branch 83.1% (1815/2184), and method 99.2% (265/267). The HTML,
text, and Cobertura report are local artifacts under
`artifacts/coverage/20261028-133721-4738372/`.

The FsCheck property `PreservesRandomizedComparisonWidths` ran 100 cases with
replay seed `281028,44017`; it varies bit widths 1–256 and comparison constants
within the exact destination domain.

## Explicit signed-state expansion

The prospective fixture grammar now accepts an explicit Boolean `signed` signal
field. It lowers signed state arithmetic and reset zero as signed fixed-width
literals, rejects any implicit signed/unsigned arithmetic mix, and preserves the
strict current-VeloSim boundary by rejecting even a redundant signedness field
in `velosim.uhdm.normalized.v1` input. Signed comparisons are not part of this
envelope version.

On 2026-10-28, the focused bridge suite ran 19 passed, 0 failed, 0 skipped
tests, including signed lowering, end-to-end model binding/BMC, mixed-sort
rejection, and current-producer rejection. Coverlet and ReportGenerator wrote
`artifacts/coverage/signed-stage1-focused-20261028/`; within that focused
scope, VeloHdl line coverage was 90.8%. The all-assembly 51.7% line coverage
from that report is intentionally not compared with the prior full-suite 96.3%
metric, because the test scope differs. The unfiltered collector run exceeded
the local 30-second command observation limit and is not claimed as a result.

The signed-comparison extension was then checked with 20 focused bridge tests
passing and no failures or skips. Its Coverlet/ReportGenerator artifact is
`artifacts/coverage/signed-comparison-focused-20261028/`, with 51.9% line and
40.2% branch coverage across all three assemblies under the focused scope.
The extension is exercised through translation, VeloFormal binding, and bounded
Z3 checking; it does not constitute a Yosys differential result.

Negative signed constant lowering was checked with 21 focused bridge tests
passing and no failures or skips. It accepts only the declared mathematical
range for each signed destination and emits fixed-width unary two's-complement
expressions. The focused Coverlet/ReportGenerator artifact is
`artifacts/coverage/signed-constants-focused-20261028/`: 52.8% line and 41.2%
branch coverage across all measured assemblies. This remains a focused-scope
metric and pre-differential evidence only.

Unary signed negation was exercised through structural translation, model
binding, and BMC with 22 focused bridge tests passing. Coverlet and
ReportGenerator produced
`artifacts/coverage/unary-negation-focused-20261028/`: 52.9% line and 41.4%
branch coverage across all measured assemblies in that focused scope. It is not
comparable to full-suite coverage and provides no external differential result.

Explicit fixture reset constants were exercised with 23 focused bridge tests
passing, including a negative signed reset value. The current VeloSim
normalized-v1 producer continues to reject the same nonzero-reset input,
preserving its documented zero-reset boundary. The focused
Coverlet/ReportGenerator artifact is
`artifacts/coverage/reset-constants-signed-focused-20261028/`: 52.9% line and
41.4% branch coverage across all measured assemblies. It remains
pre-differential evidence only.

The paired Stage-1 corpus was added with 24 focused bridge tests passing. The
prospective signed comparison fixture binds its checked-in SystemVerilog source,
normalized fixture artifact, and VeloFormal contract before BMC; no claim is
made that VeloSim emitted that prospective artifact. Focused Coverlet and
ReportGenerator evidence is under
`artifacts/coverage/stage1-corpus-focused-20261028/`: 52.9% line and 41.5%
branch coverage across the three measured assemblies. A future external run
must add version-pinned Yosys/SymbiYosys status and witnesses separately.

The prepared negative differential fixture was checked locally through the
same source/normalized provenance binding and VeloFormal BMC: 25 focused bridge
tests passed, and its `count < 2` contract produced the expected
`counterexample` status at depth 3. The corresponding `.sby` script is
checked in but was not run because SymbiYosys is unavailable. Focused coverage
is at `artifacts/coverage/stage1-negative-focused-20261028/`: 52.9% line and
41.5% branch coverage across the measured assemblies. It is not external
differential evidence.

The C# `yosys-stage1` runner was added and executed through its unavailable-tool
path on 2026-10-28. It returned exit 2 and wrote
`veloformal.yosys-stage1.v1` evidence with hashes of `counter.sv` and
`counter-bound.sby`; no SBY result was claimed. The focused Build-options and
bridge suite had 48 passing tests. Its Coverlet/ReportGenerator result is
`artifacts/coverage/yosys-stage1-runner-focused-20261028/`: 50.2% line and
41.4% branch coverage across four assemblies. `VeloFormal.Build.YosysStage1`
had 11.8% focused line coverage: fixture-preflight and unavailable-tool paths
are exercised, while the real SBY completion/status paths remain unmeasured
until a toolchain is available.

The prepared positive signed fixture then produced `bounded-pass` locally from
its source-bound normalized artifact and matching VeloFormal contract. Its
paired SBY script remains unexecuted because SBY is unavailable. The 26-test
focused bridge run produced
`artifacts/coverage/stage1-positive-focused-20261028/`: 47.8% line and 39.3%
branch coverage across the four measured assemblies. The lower aggregate metric
is a scope artifact (the Build helper assembly is included); it is not
comparable to earlier three-assembly or full-suite figures.

The SharpFuzz target was extended to feed arbitrary UTF-8 to the
provenance-bound normalized VeloHdl translator in addition to the contract
parser. `dotnet build fuzz/VeloFormal.Fuzz/VeloFormal.Fuzz.csproj -c Release
--no-restore` succeeded on 2026-10-28. No coverage-guided AFL++ or libFuzzer
campaign was run on this Windows host, so there is no fuzz execution count,
seed, corpus, or coverage result to claim.

The FsCheck property `RejectsRandomNormalizedTextWithoutUnexpectedExceptions`
ran 100 generated cases with replay seed `281028,13057`. It exercises random
text through synthetic-provenance creation and the strict normalized translator;
only its public `InvalidDataException` rejection is accepted. The 27-test
focused suite produced
`artifacts/coverage/normalized-property-focused-20261028/`: 47.8% line and
39.3% branch coverage across four assemblies. This deterministic property test
complements but does not substitute for the unrun coverage-guided fuzz campaign.

## Differential limitation

At inspection, commands `yosys`, `yosys-smtbmc`, `sby`, `iverilog`,
`verilator`, and `slang` were unavailable on this host. No external solver,
simulator, elaborator, trace, pass/fail comparison, or performance result is
claimed. Install a version-pinned environment and execute the Stage-1 matrix in
`docs/yosys-compatibility-program.md` before claiming Yosys compatibility.

The reproducible C# probe was subsequently run with:

```powershell
dotnet run --project tools/VeloFormal.Build -c Release --no-build -- yosys-probe
```

It recorded 0/4 available tools (`yosys`, `yosys-smtbmc`, `sby`, and `slang`)
in `artifacts/yosys-probe/20261028-134628-2522426/toolchain.json`. That local,
ignored artifact captures the executable configuration and absence result; it
is not a formal or differential result.

## Simulated peer-review follow-up

A simulated review found and the implementation corrected: CLI provenance
mislabeling between VeloSim and the prospective fixture grammar, use of a
bitvector state as a Boolean control, Boolean/bitvector mixing in one-bit
comparisons, text-only arithmetic tests, and overstated current-VeloSim scope.
The public CLI now has separate VeloSim and VeloHdl fixture translation commands;
the latter is the only command accepting `velohdl.normalized.v1`. This remains
an internal simulated review, not independent Yosys evidence.
