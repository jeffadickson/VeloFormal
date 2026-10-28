# Parser fuzzing

`VeloFormal.Fuzz` is a SharpFuzz target for the public contract parser. It treats
`LanguageException` as expected malformed input and exposes unexpected exceptions
as crashes. The target is not part of ordinary xUnit execution.

Run it from a Linux or WSL environment with AFL++ or libFuzzer configured for
SharpFuzz. Retain the exact fuzzer version, corpus, seed, execution time, and
minimized crash input with every campaign result. Windows unit tests verify parser
limits but are not a substitute for coverage-guided fuzzing.
