using SharpFuzz;
using VeloHdl;
using VeloFormal.Language;

// AFL++/libFuzzer supplies arbitrary UTF-8 text. Expected contract and
// normalized-RTL rejections are consumed; any other exception is a finding.
Fuzzer.OutOfProcess.Run(input =>
{
    try
    {
        ContractParser.Parse(input, "<fuzz>");
    }
    catch (LanguageException)
    {
        // Rejection is the intended behavior for malformed language input.
    }

    try
    {
        var provenance = StructuralProvenance.Create("fuzz", "Fuzz", "1", input);
        VeloSimNormalizedTranslator.Translate(input, provenance, "fuzz");
    }
    catch (InvalidDataException)
    {
        // The normalized VeloHdl grammar is intentionally fail-closed.
    }
});
