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
