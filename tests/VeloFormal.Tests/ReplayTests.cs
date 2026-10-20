using System.Numerics;
using System.Text.Json;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ReplayTests
{
    public static IEnumerable<object[]> Pipelines()
    {
        int iteration = 381;
        foreach (int length in new[] { 1, 2, 4, 8, 16 })
            foreach (bool mutate in new[] { false, true }) yield return [iteration++, length, mutate];
    }
    [Theory, MemberData(nameof(Pipelines))]
    public void ReplayDetectsMutations(int iteration, int length, bool mutate)
    {
        Assert.InRange(iteration, 381, 390);
        int delay = length % 3 + 1;
        var samples = Enumerable.Range(0, length + delay + 1).Select(t => new Sample(new Dictionary<string, BigInteger>
        {
            ["count"] = mutate && t == delay ? -1 : t,
            ["write"] = t < length ? 1 : 0
        })).ToArray();
        var response = Expression.Compile($"count == at_trigger(count) + {delay}", true);
        var contract = new Contract(ensures: [new("advance", s => s["write"] != 0, (s, t) => response(s, t), delay, delay)]);
        using var encoded = new StringWriter();
        TraceWriter.WriteJsonLines(encoded, samples);
        using var reader = new StringReader(encoded.ToString());
        var report = TraceChecker.Check(contract, SampleStream.ReadJsonLines(reader));
        Assert.Equal(mutate ? "counterexample" : "trace-pass", report.Status);
        Assert.Equal(0, report.Pending);
        if (mutate) Assert.Contains(report.Findings, f => f.TriggerSample == 0 && f.Sample == delay);
        else Assert.Empty(report.Findings);
    }

    public static IEnumerable<object[]> Streams()
    {
        string[] names = ["file", "stdin", "contract_first", "assumption_stop", "malformed", "nonempty_contract",
            "duplicate", "unknown", "missing_value", "empty"];
        return names.Select((name, index) => new object[] { 391 + index, name });
    }
    [Theory, MemberData(nameof(Streams))]
    public void StreamingCli(int iteration, string scenario)
    {
        Assert.InRange(iteration, 391, 400);
        string directory = Path.Combine(Path.GetTempPath(), "velo stream " + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string contract = Path.Combine(directory, "contract.trace"), samples = Path.Combine(directory, "samples.jsonl");
            string json = scenario == "assumption_stop"
                ? """{"contract":{"requires":{"legal":"False"}},"samples":[]}"""
                : """{"contract":{"invariants":{"safe":"n >= 0"}},"samples":[]}""";
            if (scenario == "nonempty_contract") json = json.Replace("\"samples\":[]", "\"samples\":[{}]");
            string text = scenario == "empty" ? "" : scenario == "malformed" ? "{bad}"
                : scenario == "assumption_stop" ? "{\"n\":0}\n{bad}" : "{\"n\":0}\n{\"n\":1}";
            File.WriteAllText(contract, json); File.WriteAllText(samples, text);
            string[] args = ["--samples", scenario == "stdin" ? "-" : samples, "--contract", contract];
            if (scenario == "contract_first") args = ["--contract", contract, "--samples", samples];
            if (scenario == "duplicate") args = ["--samples", samples, "--samples", samples];
            if (scenario == "unknown") args = ["--samples", samples, "--unknown", contract];
            if (scenario == "missing_value") args = ["--samples", samples, "--contract"];
            using var output = new StringWriter(); using var error = new StringWriter();
            int code = Command.Run(args, new StringReader(text), output, error);
            int expected = scenario is "malformed" or "nonempty_contract" or "duplicate" or "unknown" or "missing_value" ? 2
                : scenario is "assumption_stop" or "empty" ? 1 : 0;
            Assert.Equal(expected, code);
            if (expected == 2) Assert.Empty(output.ToString());
            else
            {
                using var report = JsonDocument.Parse(output.ToString());
                Assert.Equal(scenario == "assumption_stop" ? "invalid-assumptions" : scenario == "empty" ? "incomplete" : "trace-pass",
                    report.RootElement.GetProperty("status").GetString());
            }
        }
        finally { Directory.Delete(directory, recursive: true); }
    }
}
