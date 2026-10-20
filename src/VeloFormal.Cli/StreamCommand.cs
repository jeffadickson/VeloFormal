namespace VeloFormal;

public static class StreamCommand
{
    public static Report Check(string[] args, TextReader input)
    {
        var options = new Dictionary<string, string>(StringComparer.Ordinal);
        if (args.Length != 4) throw new InputException("expected --samples FILE --contract FILE");
        for (int index = 0; index < args.Length; index += 2)
        {
            if (string.IsNullOrWhiteSpace(args[index + 1]) || args[index] is not "--samples" and not "--contract" || !options.TryAdd(args[index], args[index + 1]))
                throw new InputException("invalid or duplicate streaming option");
        }
        if (!options.TryGetValue("--samples", out var path) || !options.TryGetValue("--contract", out var contractPath)
            || contractPath == "-") throw new InputException("samples and contract file are required");
        var document = TraceDocument.Parse(File.ReadAllText(contractPath));
        if (document.Samples.Count != 0) throw new InputException("streaming contract must have empty samples");
        if (path == "-") return TraceChecker.Check(document.Contract, SampleStream.ReadJsonLines(input));
        using var reader = File.OpenText(path);
        return TraceChecker.Check(document.Contract, SampleStream.ReadJsonLines(reader));
    }
}
