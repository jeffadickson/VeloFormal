using System.CommandLine;

namespace VeloFormal;

/// <summary>
/// Declares the public CLI vocabulary through System.CommandLine while the legacy
/// command adapter continues to own I/O and exit-code compatibility.
/// </summary>
internal static class CliSchema
{
    /// <summary>Rejects unknown options and invalid option arity before command dispatch.</summary>
    internal static void Validate(string[] args)
    {
        var root = new RootCommand("VeloFormal contract and trace checker");
        foreach (string option in new[]
        {
            "--trace", "--samples", "--contract", "--vcd", "--clock",
            "--parse-contract", "--format-contract", "--check-contract", "--model",
            "--depth", "--timeout-ms", "--max-queries", "--translate-velosim-normalized", "--translate-velohdl-normalized",
            "--module", "--source", "--normalized-ir", "--frontend-revision", "--verify-velohdl"
        })
            root.Options.Add(new Option<string>(option));
        // Waveform checking deliberately accepts several distinct signal mappings.
        root.Options.Add(new Option<string[]>("--signal"));
        root.Options.Add(new Option<bool>("--demo"));
        var result = root.Parse(args);
        if (result.Errors.Count != 0) throw new InputException(result.Errors[0].Message);
    }
}
