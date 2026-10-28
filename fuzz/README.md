# Parser fuzzing

`VeloFormal.Fuzz` is a SharpFuzz target for the public contract parser and the
VeloHdl normalized-RTL translator. It treats `LanguageException` and
`InvalidDataException` as expected malformed input and exposes unexpected
exceptions as crashes. The translator input is bound to per-input synthetic
provenance so its hash-validation path is exercised too. The target is not part
of ordinary xUnit execution.

Run it from a Linux or WSL environment with AFL++ or libFuzzer configured for
SharpFuzz. Retain the exact fuzzer version, corpus, seed, execution time, and
minimized crash input with every campaign result. Windows unit tests verify parser
limits but are not a substitute for coverage-guided fuzzing.
