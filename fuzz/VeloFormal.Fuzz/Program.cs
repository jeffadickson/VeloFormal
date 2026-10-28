using SharpFuzz;
using VeloFormal.Language;

// AFL++/libFuzzer supplies arbitrary UTF-8 text. Expected syntax failures are
// consumed; any other exception is a fuzzing finding.
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
});
