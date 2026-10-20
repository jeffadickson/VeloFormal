using VeloFormal;

namespace VeloFormal;

public static class WaveformCommand
{
    public static TraceDocument Read(string[] args, TextReader input)
    {
        var options = new Dictionary<string, string>(StringComparer.Ordinal);
        var signals = new Dictionary<string, string>(StringComparer.Ordinal);
        for (int index = 0; index < args.Length; index += 2)
        {
            string key = args[index];
            if (key is not "--vcd" and not "--contract" and not "--clock" and not "--signal")
                throw new InputException($"unknown waveform option: {key}");
            if (index + 1 == args.Length || string.IsNullOrWhiteSpace(args[index + 1]))
                throw new InputException($"missing value for {key}");
            string value = args[index + 1];
            if (key == "--signal")
            {
                var mapping = value.Split('=');
                if (mapping.Length != 2 || !ValidName(mapping[0]) || string.IsNullOrWhiteSpace(mapping[1]))
                    throw new InputException("signal mapping must be NAME=hierarchical.path");
                if (!signals.TryAdd(mapping[0], mapping[1])) throw new InputException($"duplicate signal mapping: {mapping[0]}");
            }
            else if (!options.TryAdd(key, value)) throw new InputException($"duplicate waveform option: {key}");
        }
        foreach (string key in new[] { "--vcd", "--contract", "--clock" })
            if (!options.ContainsKey(key)) throw new InputException($"missing waveform option: {key}");
        if (signals.Count == 0) throw new InputException("select at least one --signal");
        if (options["--contract"] == "-") throw new InputException("contract must be a file");
        var contract = TraceDocument.Parse(File.ReadAllText(options["--contract"]));
        if (contract.Samples.Count != 0) throw new InputException("waveform contract document must have an empty samples array");
        string text = options["--vcd"] == "-" ? input.ReadToEnd() : File.ReadAllText(options["--vcd"]);
        return new(contract.Contract, VcdReader.Read(text, options["--clock"], signals));
    }
    private static bool ValidName(string name) => name.Length > 0 && char.IsAsciiLetter(name[0])
        && name.All(c => char.IsAsciiLetterOrDigit(c) || c == '_')
        && name is not "True" and not "False" and not "and" and not "or" and not "not" and not "at_trigger";
}
