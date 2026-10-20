using System.Numerics;
using Xunit;
namespace VeloFormal.Tests;
public sealed class TraceWriterTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["empty", "zero", "one", "negative", "wide_positive", "wide_negative", "int_max", "int_min", "sorted", "roundtrip",
            "two_lines", "writer_ownership", "null_sample", "invalid_name", "unicode_name", "reserved_name", "cancel_before", "cancel_mid", "snapshot", "boolean_canonical"];
        return names.Take(10).Select((name, index) => new object[] { 341 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void ExportSamples(int iteration, string scenario)
    {
        Assert.InRange(iteration, 341, 360);
        var values = new Dictionary<string, BigInteger>();
        BigInteger number = scenario switch
        {
            "one" => 1, "negative" => -1, "wide_positive" => BigInteger.One << 130,
            "wide_negative" => -(BigInteger.One << 130), "int_max" => int.MaxValue, "int_min" => int.MinValue, _ => 0
        };
        if (scenario != "empty") values["n"] = number;
        if (scenario == "sorted") { values["z"] = 1; values["a"] = 2; }
        if (scenario == "invalid_name") values["bad name"] = 0;
        if (scenario == "unicode_name") values["λ"] = 0;
        if (scenario == "reserved_name") values["True"] = 0;
        var sample = scenario == "boolean_canonical" ? TraceDocument.ParseSample("""{"n":true}""") : new Sample(values);
        if (scenario is "invalid_name" or "unicode_name" or "reserved_name")
        {
            Assert.Throws<InputException>(() => TraceWriter.SerializeSample(sample)); return;
        }
        if (scenario == "null_sample") { Assert.Throws<ArgumentNullException>(() => TraceWriter.SerializeSample(null!)); return; }
        using var cancellation = new CancellationTokenSource();
        using var writer = new StringWriter();
        if (scenario == "cancel_before")
        {
            cancellation.Cancel();
            Assert.Throws<OperationCanceledException>(() => TraceWriter.WriteJsonLines(writer, [sample], cancellation.Token)); return;
        }
        if (scenario == "cancel_mid")
        {
            IEnumerable<Sample> Samples() { yield return sample; cancellation.Cancel(); yield return sample; }
            Assert.Throws<OperationCanceledException>(() => TraceWriter.WriteJsonLines(writer, Samples(), cancellation.Token)); return;
        }
        if (scenario == "snapshot") values["n"] = 999;
        TraceWriter.WriteJsonLines(writer, scenario == "two_lines" ? [sample, sample] : [sample]);
        var parsed = SampleStream.ReadJsonLines(new StringReader(writer.ToString())).ToArray();
        Assert.Equal(scenario == "two_lines" ? 2 : 1, parsed.Length);
        foreach (var (name, expected) in sample.Values) Assert.Equal(expected, parsed[0][name]);
        if (scenario == "sorted") Assert.StartsWith("{\"a\":2,\"n\":0,\"z\":1}", writer.ToString());
        if (scenario == "boolean_canonical") Assert.Contains("\"n\":1", writer.ToString());
        if (scenario == "writer_ownership") { writer.Write("still open"); Assert.EndsWith("still open", writer.ToString()); }
    }
}
