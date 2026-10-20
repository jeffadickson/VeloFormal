using System.Text.Json;
using Xunit;

namespace VeloFormal.Tests;

public sealed class InputTests
{
    public static IEnumerable<object[]> InvalidExpressions()
    {
        string[] sources = ["", " ", "a.value", "a[0]", "f()", "__import__('os')",
            "[1]", "lambda: True", "1 * 2", "1 / 2", "1 // 2", "1 % 2", "2 ** 3",
            "1 << 2", "2 >> 1", "a ? b : c", "1 = 1", "1 +", "(1", "1)",
            "at_trigger(at_trigger(count))", "not", "1,2", "0x10", "\"text\""];
        return sources.Select((source, index) => new object[] { index + 101, source });
    }
    [Theory]
    [MemberData(nameof(InvalidExpressions))]
    public void RejectUnsupportedExpressions(int iteration, string source)
    {
        Assert.InRange(iteration, 101, 125);
        Assert.Throws<InputException>(() => Expression.Compile(source, true));
    }

    public static IEnumerable<object[]> InvalidDocuments()
    {
        string[] documents = [
            "null", "[]", "{}", "{", 
            """{"contract":{},"samples":[],"extra":0}""",
            """{"contract":{},"contract":{},"samples":[]}""",
            """{"contract":{"clock":"clk"},"samples":[]}""",
            """{"contract":{"requires":[]},"samples":[]}""",
            """{"contract":{"ensures":{}},"samples":[]}""",
            """{"contract":{"ensures":[{"label":"a","when":"True","response":"True","after":1,"within":[1,2]}]},"samples":[]}""",
            """{"contract":{"ensures":[{"label":"a","when":"True","response":"True","after":true}]},"samples":[]}""",
            """{"contract":{"invariants":{"a":"True"},"covers":{"a":"True"}},"samples":[]}""",
            """{"contract":{},"samples":[{"a":0,"a":1}]}""",
            """{"contract":{},"samples":[{"a":"X"}]}""",
            """{"contract":{},"samples":[{"a":1.5}]}"""
        ];
        return documents.Select((document, index) => new object[] { index + 126, document });
    }
    [Theory]
    [MemberData(nameof(InvalidDocuments))]
    public void RejectMalformedDocuments(int iteration, string document)
    {
        Assert.InRange(iteration, 126, 140);
        Assert.Throws<InputException>(() => TraceDocument.Parse(document));
    }

    public static IEnumerable<object[]> CliCases() => new object[][]
    {
        [141, "help", 0], [142, "demo", 0], [143, "unknown", 2], [144, "pass", 0],
        [145, "counterexample", 1], [146, "incomplete", 1], [147, "invalid-assumptions", 1],
        [148, "malformed", 2], [149, "missing_signal", 2], [150, "missing_file", 2]
    };
    [Theory]
    [MemberData(nameof(CliCases))]
    public void CliReportsStatus(int iteration, string scenario, int expected)
    {
        Assert.InRange(iteration, 141, 150);
        string[] args = ["--trace", "-"];
        string json = scenario switch
        {
            "pass" => """{"contract":{},"samples":[{}]}""",
            "counterexample" => """{"contract":{"invariants":{"bad":"False"}},"samples":[{}]}""",
            "invalid-assumptions" => """{"contract":{"requires":{"bad":"False"}},"samples":[{}]}""",
            "missing_signal" => """{"contract":{"invariants":{"needs_count":"count > 0"}},"samples":[{}]}""",
            "incomplete" => """{"contract":{},"samples":[]}""",
            _ => "{"
        };
        if (scenario == "help") args = ["--help"];
        if (scenario == "demo") args = ["--demo"];
        if (scenario == "unknown") args = ["--unknown"];
        if (scenario == "missing_file") args = ["--trace", Path.Combine(Path.GetTempPath(), Guid.NewGuid() + ".trace")];
        using var output = new StringWriter();
        using var error = new StringWriter();
        Assert.Equal(expected, Command.Run(args, new StringReader(json), output, error));
        if (expected == 2)
        {
            Assert.Empty(output.ToString());
            Assert.StartsWith("veloformal:", error.ToString());
            if (scenario == "missing_signal") Assert.Contains("clause needs_count, sample 0", error.ToString());
        }
        else if (scenario != "help")
        {
            Assert.Empty(error.ToString());
            using var report = JsonDocument.Parse(output.ToString());
            Assert.Equal(scenario == "demo" || scenario == "pass" ? "trace-pass" : scenario,
                report.RootElement.GetProperty("status").GetString());
        }
    }
}
