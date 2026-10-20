using System.Text.Json;
using Xunit;

namespace VeloFormal.Tests;

public sealed class IntegrationTests
{
    [Theory]
    [InlineData(191, 1, false, "trace-pass")]
    [InlineData(192, 2, false, "counterexample")]
    [InlineData(193, 1, true, "trace-pass")]
    [InlineData(194, 2, true, "trace-pass")]
    [InlineData(195, 2147483647, false, "incomplete")]
    public void TraceFileCli(int iteration, int delay, bool reset, string status)
    {
        Assert.InRange(iteration, 191, 195);
        var ensure = new Dictionary<string, object> { ["label"] = "rise", ["when"] = "write",
            ["response"] = "count == at_trigger(count) + 1", ["after"] = delay };
        var document = new { contract = new { reset = "reset", ensures = new[] { ensure } },
            samples = new[] { new { count = 0, write = true, reset = false },
                             new { count = 1, write = false, reset },
                             new { count = 0, write = false, reset = false } } };
        var file = Path.Combine(Path.GetTempPath(), Guid.NewGuid() + ".trace");
        try
        {
            File.WriteAllText(file, JsonSerializer.Serialize(document));
            using var output = new StringWriter(); using var error = new StringWriter();
            int code = Command.Run(["--trace", file], TextReader.Null, output, error);
            Assert.Equal(status == "trace-pass" ? 0 : 1, code);
            using var report = JsonDocument.Parse(output.ToString());
            Assert.Equal(status, report.RootElement.GetProperty("status").GetString());
            Assert.Empty(error.ToString());
        }
        finally { File.Delete(file); }
    }

    [Theory]
    [InlineData(196, "covers", "True", 1)]
    [InlineData(197, "covers", "False", 0)]
    [InlineData(198, "invariants", "-1 < 0 and ~0 == -1", 0)]
    [InlineData(199, "requires", "True", 0)]
    [InlineData(200, "invariants", "1 + 2 & 1 == 1", 0)]
    public void ReportThroughJson(int iteration, string role, string expression, int covers)
    {
        Assert.InRange(iteration, 196, 200);
        string json = JsonSerializer.Serialize(new { contract = new Dictionary<string, object>
            { [role] = new Dictionary<string, string> { ["clause"] = expression } }, samples = new[] { new { }, new { } } });
        var document = TraceDocument.Parse(json);
        var report = TraceChecker.Check(document.Contract, document.Samples);
        Assert.Equal("trace-pass", report.Status);
        Assert.Equal(covers, report.Covered.Count);
        Assert.Equal(2, report.ActiveSamples);
    }
}
