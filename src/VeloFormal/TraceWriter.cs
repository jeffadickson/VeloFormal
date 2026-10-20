using System.Globalization;
using System.Text;
using System.Text.Json;

namespace VeloFormal;
public static class TraceWriter
{
    public static string SerializeSample(Sample sample)
    {
        ArgumentNullException.ThrowIfNull(sample);
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            writer.WriteStartObject();
            foreach (var (name, value) in sample.Values.OrderBy(p => p.Key, StringComparer.Ordinal))
            {
                if (!TraceDocument.SignalName(name)) throw new InputException($"invalid signal name: {name}");
                writer.WritePropertyName(name);
                writer.WriteRawValue(value.ToString(CultureInfo.InvariantCulture));
            }
            writer.WriteEndObject();
        }
        return Encoding.UTF8.GetString(stream.ToArray());
    }
    /// <summary>Writes samples only; no delegate-based contract serialization. Does not close the writer.</summary>
    public static void WriteJsonLines(TextWriter writer, IEnumerable<Sample> samples, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(writer);
        ArgumentNullException.ThrowIfNull(samples);
        cancellationToken.ThrowIfCancellationRequested();
        foreach (var sample in samples)
        {
            cancellationToken.ThrowIfCancellationRequested();
            writer.WriteLine(SerializeSample(sample));
        }
        cancellationToken.ThrowIfCancellationRequested();
    }
}
