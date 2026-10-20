# Recorded waveform and streaming samples

C# commands (Python is not required):

```console
dotnet run --project src/VeloFormal.Cli -- --vcd recording.vcd --contract examples/waveform.contract --clock tb.clk --signal data=tb.data
dotnet run --project src/VeloFormal.Cli -- --samples examples/samples.jsonl --contract examples/stream.contract
dotnet run --project tools/VeloFormal.Build -c Release -- coverage
```

For waveform stdin use `--vcd -`; for JSONL stdin use `--samples -`.
Contract documents use the existing schema and must have an empty `samples`
array, preventing accidental mixing of two trace sources. Repeat `--signal`
for multiple bindings. Signal aliases use the same names as expression inputs.
Report sample indices refer to selected rising edges, not VCD timestamps;
`VcdReader.ReadFrames` preserves original timestamps in API results. A declared
clock with no observed value or no known 0-to-1 edge gives an empty observation
and an incomplete report. Clock X/Z is an input error. Sampling observes settled
timestamp values rather than SVA preponed values.

JSONL consists of one sample object per nonblank physical line. Reader ownership
stays with the caller in `SampleStream.ReadJsonLines`. Malformed lines report
their physical line number. Checking stops at the first failed requirement;
unread trailing lines are not validated. The 1 MiB line limit is a validation
limit checked after reading a line, not a preallocation memory bound.

`CheckOptions` defaults to 1,000,000 observed samples, 100,000 pending obligations,
and 100,000 findings. Limits must be positive and exceeding one throws an input
error; results are never truncated into a pass. Eager JSON/VCD parsing is outside
these checker limits. The API accepts a cancellation token; cancellation throws
rather than returning proof evidence. User predicates and readers are synchronous
and must return for cancellation to be observed.

Reports include observed/reset sample counts and pending obligation labels,
trigger samples, earliest response samples and deadlines. Deadline values use
64-bit indices. Immutable samples and checker-generated report collections
prevent later input mutation from changing recorded evidence.

`Expression.Describe` gives sorted, distinct current/trigger signal references.
These are syntactic references, including short-circuited branches, not a
guarantee that a signal is read on every sample. `TraceWriter.WriteJsonLines`
exports samples with deterministic field order and exact integer values.
Booleans are canonicalized to 0/1. It leaves the writer open and does not serialize
delegate-based contracts.

The C# build helper accepts `--no-build` to use existing binaries and
`--no-restore` to skip project/tool restoration. These flags do not apply to
`rtl` mode. To avoid even the outer `dotnet run` rebuild, pass its
`--no-build` before the argument separator as well.

Finite trace results are observations, not formal proofs.
