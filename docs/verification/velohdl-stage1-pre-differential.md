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

The portable-suite runner path was then rebuilt and its unavailable-tool path
re-executed: Build-option tests had 23 passes and the runner exited 2 with a
new diagnostic artifact, as expected. The narrow helper-only coverage artifact
is `artifacts/coverage/portable-sby-runner-focused-20261028/`; it reports
23.8% line coverage for `VeloFormal.Build` and 11.2% for `YosysStage1`.
The absolute-SBY child-process branch remains unmeasured because no portable
suite has been extracted on this host.

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

## First pinned external fixture executions

On 2026-10-28, an isolated official OSS CAD Suite Windows x64 archive tagged
`2026-10-28` was extracted outside the checkout. Its archive SHA-256 was
`eb97af9e5d0b195e570c6a7ee3d29c005bcc60e343d48cb713ef414e4a39e766`.
The probe artifact `artifacts/yosys-probe/20261028-143956-0230970/toolchain.json`
recorded Yosys, `yosys-smtbmc`, and SBY as available, and Slang as unavailable.
Yosys reported `0.70+255` (`72b0054f0`); SBY reported `v0.70-9-g79fc9ed`.
The suite's `bin` and `lib` directories were supplied only to child processes;
the system-wide `PATH` was not changed.

The Stage-1 runner then executed `counter-bound.sby` using BMC depth 3, Z3,
and the source-bound counter fixture. It wrote
`artifacts/yosys-stage1/20261028-144042-8044110/result.json`, whose source hash
is `aed71e167fdfa642606c97b57b22bfdc771e627cb7f8b5ecbabf1c27a64bc709` and
whose SBY-script hash is
`087cd4aae8daec7d1e7d0fd73ec7cdb372d695c1415ac18910b7e056209a6b4b`.
SBY returned its expected bounded `FAIL` verdict (raw status `FAIL 2 0`, process
exit 2) and retained `trace.vcd`. The runner now parses the leading verdict
token while retaining the raw line, because current SBY adds task counters.

The same bounded VeloFormal check reported `counterexample` at depth 3. It
bound the same source hash and a normalized IR hash of
`c091c147b9b66c6b2ccf86c50009e88bfb58c77f15f9ace2c343697f517bbaa0` in
`artifacts/differential/20261028-144200/counter-veloformal.json`. Both engines
therefore agree on the requested counterexample verdict. Their captured initial
counter values differ (`253` in the SBY VCD and `128` in the VeloFormal model)
because the property permits an unconstrained initial state; verdict agreement
does not establish an identical witness. A constrained-init replay fixture is
still required before claiming counterexample trace equivalence.

The positive `signed-compare-pass.sby` job used the same BMC depth and Z3
configuration and returned `PASS 0 0`. It was first observed in
`artifacts/yosys-stage1/positive-20261028-144100/status` and then reproduced by
the C# `yosys-stage1-positive` runner in
`artifacts/yosys-stage1/20261028-144933-4155033/result.json`. Its source-bound
VeloFormal counterpart reported `bounded-pass` at depth 3, with source hash
`fb9b888f7b7e87e83d882045aa63553cbf892f0e3fe29b5423ad6923b731f113` and
normalized IR hash
`318c2cf578eae9dc19b2b0b4cee89b18e3aea671437742c28714c20568da9b3d`.
This is a pass/pass bounded-status comparison, not a proof and not evidence
that the prospective normalized artifact was emitted by VeloSim.

These are the first external Stage-1 fixture results. They qualify neither the
whole Stage-1 profile nor a general Yosys-compatible frontend: only two small,
source-pinned fixtures have matched bounded statuses, and the current VeloSim
checkout remains dirty and unqualified as a producer.

## Initialized counterexample replay fixture

The prospective fixture grammar now permits an optional complete `initial`
state assignment. It rejects partial assignments and keeps the field outside
the current VeloSim normalized-v1 boundary. This supports a source-pinned
`counter-init` fixture whose SystemVerilog `initial count = 8'd0` and FORMAL
assumption `rst_n` match the normalized initial assignment and VeloFormal
`require reset_inactive: rst_n` clause.

The release build and focused Build-option/bridge run completed with 52 passed,
0 failed, and 0 skipped tests. The VeloFormal check at source-state depth 3
wrote `artifacts/differential/20261028-145310-counter-init/` and reported a
counterexample at sample 2 with the exact prefix `(rst_n,count)` of `(1,0)`,
`(1,1)`, `(1,2)`. Its source hash is
`db6acfa8234e613ae99eb92a417d3682edafcb7076485703548f6c769108d209` and its
normalized IR hash is
`b56970e23a72e5c1b764f68f3ed08789723bb8ca4f68595421df85fb2440b39b`.

The `yosys-stage1-deterministic` runner executed the same source and
`counter-init-bound.sby` through the pinned SBY/Z3 suite. Its result is
`artifacts/yosys-stage1/20261028-145233-4781866/result.json`; it reports
expected `FAIL` (raw status `FAIL 2 0`), source hash
`db6acfa8234e613ae99eb92a417d3682edafcb7076485703548f6c769108d209`, and
script hash `2b931bb75bd764d529c6f4012ca3d708f2c31f8a3d564671a6218a337bfe5fdd`.
The SBY VCD has `(rst_n,count)` `(1,0)`, `(1,1)`, `(1,2)`, `(1,3)` at steps
0 through 3. Its failed clocked assertion at step 3 sampled the pre-NBA value
`count=2`; the VCD records the post-NBA value `count=3`. Therefore the first
three VCD states replay the VeloFormal trace exactly, while the final VCD state
is its required post-update observation. This one-step horizon/scheduling
normalization is explicit fixture metadata, not an unexamined verdict match.

## Initialized signed comparison and negation replay fixture

`signed-init` extends the deterministic corpus with signed eight-bit state,
an explicit signed comparison, a mux (`ite`), unary negation, and signed
subtraction. It initializes `count` to mathematical `-1`, constrains `rst_n`,
and deliberately violates `count <= 0` after the negative branch computes
`-(-1) = 1`. The local VeloFormal test reports the encoded fixed-width prefix
`255` (signed `-1`) then `1`, so the mathematical signed interpretation and
stored bit pattern are both explicit.

The release build and focused Build-option/bridge tests completed with 53
passed, 0 failed, and 0 skipped tests. The pinned external
`yosys-stage1-signed` runner produced expected `FAIL` at
`artifacts/yosys-stage1/20261028-145455-8602518/result.json`, with source hash
`3f9be8ea5f9be040b20585dd14d0bfef34f2437c1505c962ff8fa23e368b1af9`, SBY
script hash `fd4beccba013e5918427336d02e63e9523758c54d67a13e7f40798f7eba791a6`,
and raw status `FAIL 2 0`. Its VCD begins with signed bit patterns `0xff`,
`0x01`, and `0x00`, matching the VeloFormal prefix through the same pre-NBA
clocked-assertion convention recorded for `counter-init`.

## Concurrent nonblocking-update replay fixture

`two-registers-init` is the first paired external fixture with two concurrent
state processes. From complete initialization `(a,b)=(0,0)` and constrained
inactive reset, it updates `a` to `a + 1` while independently assigning
`b <= a ^ 3`. Both the VeloFormal counterexample and SBY VCD therefore show
`(a,b)=(0,0)`, `(1,3)`, `(2,2)`; the middle state distinguishes proper
nonblocking prior-state reads from an accidental sequential update.

The release build and focused Build-option/bridge tests completed with 54
passed, 0 failed, and 0 skipped tests. The pinned
`yosys-stage1-concurrent` runner wrote
`artifacts/yosys-stage1/20261028-145629-3212792/result.json` with expected
`FAIL`, raw status `FAIL 2 0`, source hash
`d5ee002acb7fb29a5371080722029cd687fca43214ae826d26293ebe34590d27`, and
SBY-script hash `f70e7571c44ad4888dae4766e3f91e3680f14e6da7ea65f31eec2ef50bb0b137`.

## Stage-1 external matrix

The C# `yosys-stage1-matrix` runner executes every currently paired fixture
serially and preserves a separate timestamped artifact for each job. On
2026-10-28 it completed successfully after a release build and the 54-pass
focused Build-option/bridge test run. The five result artifacts were
`20261028-145723-4448735` (counter `FAIL`),
`20261028-145725-3338027` (signed compare `PASS`),
`20261028-145726-3802078` (initialized counter `FAIL`),
`20261028-145727-3742165` (initialized signed control/data `FAIL`), and
`20261028-145728-4735392` (concurrent nonblocking registers `FAIL`) beneath
`artifacts/yosys-stage1/`. This is a reproducibility checkpoint for the tested
fixture set, not a claim that untested Stage-1 operators or frontends conform.

## Differential limitation

Before the pinned external run above, commands `yosys`, `yosys-smtbmc`, `sby`,
`iverilog`, `verilator`, and `slang` were unavailable from the ordinary host
environment. The portable suite supplies the first three only; there is still
no Slang, Icarus, or Verilator comparison. No general external solver,
simulator, elaborator, trace-equivalence, or performance result is claimed.
Complete the Stage-1 matrix in `docs/yosys-compatibility-program.md` before
claiming Yosys compatibility.

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
