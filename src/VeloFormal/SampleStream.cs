namespace VeloFormal;

public static class SampleStream
{
    /// <summary>Lazy JSONL samples; reader ownership remains with the caller.</summary>
    public static IEnumerable<Sample> ReadJsonLines(TextReader reader, int maxLineLength = 1048576)
    {
        ArgumentNullException.ThrowIfNull(reader);
        if (maxLineLength < 1) throw new InputException("maximum line length must be positive");
        string? line;
        int lineNumber = 0;
        while ((line = reader.ReadLine()) is not null)
        {
            lineNumber++;
            if (line.Length > maxLineLength) throw new InputException($"sample line {lineNumber} exceeds length limit {maxLineLength}");
            if (string.IsNullOrWhiteSpace(line)) continue;
            Sample sample;
            try { sample = TraceDocument.ParseSample(line); }
            catch (InputException error) { throw new InputException($"sample line {lineNumber}: {error.Message}"); }
            yield return sample;
        }
    }
}
