using System.Numerics;
using System.Text.Json;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ReportDetailTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["empty", "reset_only", "active", "mixed", "pending_label", "exact_deadline", "window", "overlap",
            "long_deadline", "reset_cancel", "satisfied", "failed", "invalid_requires", "order", "immutable_pending",
            "immutable_cover", "immutable_untriggered", "sample_snapshot", "failure_location", "cli_json"];
        return names.Select((name, index) => new object[] { 301 + index, name });
    }
    private static Sample S(int n = 0) => new(new Dictionary<string, BigInteger> { ["n"] = n });
    private static Contract C(int earliest = 1, int latest = 1) => new(ensures: [new("response", s => s["n"] == 0, (s, _) => s["n"] == 1, earliest, latest)]);
    [Theory, MemberData(nameof(Cases))]
    public void ExplainPendingWork(int iteration, string scenario)
    {
        Assert.InRange(iteration, 301, 320);
        var report = TraceChecker.Check(C(), [S()]);
        switch (scenario)
        {
            case "empty": Assert.Equal(0, TraceChecker.Check(new(), []).ObservedSamples); break;
            case "reset_only": Assert.Equal(1, TraceChecker.Check(new(reset: _ => true), [S()]).ResetSamples); break;
            case "active": Assert.Equal(3, TraceChecker.Check(new(), [S(), S(), S()]).ObservedSamples); break;
            case "mixed":
                report = TraceChecker.Check(new(reset: s => s["n"] == 0), [S(), S(1)]);
                Assert.Equal(2, report.ObservedSamples); Assert.Equal(1, report.ResetSamples); Assert.Equal(1, report.ActiveSamples); break;
            case "pending_label": Assert.Equal("response", Assert.Single(report.Obligations).Label); break;
            case "exact_deadline": Assert.Equal(1, Assert.Single(report.Obligations).DeadlineSample); break;
            case "window":
                var window = Assert.Single(TraceChecker.Check(C(2, 4), [S()]).Obligations);
                Assert.Equal(2, window.EarliestSample); Assert.Equal(4, window.DeadlineSample); break;
            case "overlap": Assert.Equal(2, TraceChecker.Check(C(3, 3), [S(), S()]).Obligations.Count); break;
            case "long_deadline":
                var longPending = Assert.Single(TraceChecker.Check(C(int.MaxValue, int.MaxValue), [S(1), S()]).Obligations);
                Assert.Equal(2147483648L, longPending.DeadlineSample); break;
            case "reset_cancel": Assert.Empty(TraceChecker.Check(new(ensures: C().Ensures, reset: s => s["n"] == 2), [S(), S(2)]).Obligations); break;
            case "satisfied": Assert.Empty(TraceChecker.Check(C(), [S(), S(1)]).Obligations); break;
            case "failed": Assert.Empty(TraceChecker.Check(C(), [S(), S(2)]).Obligations); break;
            case "invalid_requires":
                report = TraceChecker.Check(new(requires: [new("legal", s => s["n"] == 0)], ensures: C().Ensures), [S(), S(1)]);
                Assert.Equal("invalid-assumptions", report.Status); Assert.Single(report.Obligations); break;
            case "order":
                report = TraceChecker.Check(new(ensures: [new("first", _ => true, (_, _) => false), new("second", _ => true, (_, _) => false)]), [S()]);
                Assert.Equal(["first", "second"], report.Obligations.Select(o => o.Label)); break;
            case "immutable_pending": Assert.Throws<NotSupportedException>(() => ((IList<PendingObligation>)report.Obligations).Clear()); break;
            case "immutable_cover":
                report = TraceChecker.Check(new(covers: [new("seen", _ => true)]), [S()]);
                Assert.Throws<NotSupportedException>(() => ((IList<string>)report.Covered).Clear()); break;
            case "immutable_untriggered":
                report = TraceChecker.Check(C(), [S(1)]);
                Assert.Throws<NotSupportedException>(() => ((IList<string>)report.Untriggered).Clear()); break;
            case "sample_snapshot":
                var values = new Dictionary<string, BigInteger> { ["n"] = 0 }; var sample = new Sample(values);
                values["n"] = 2; Assert.Single(TraceChecker.Check(C(), [sample]).Obligations); break;
            case "failure_location":
                var finding = Assert.Single(TraceChecker.Check(C(), [S(), S(2)]).Findings);
                Assert.Equal(0, finding.TriggerSample); Assert.Equal(1, finding.Sample); break;
            case "cli_json":
                const string input = """{"contract":{"ensures":[{"label":"next","when":"True","response":"False","after":2}]},"samples":[{}]}""";
                using var output = new StringWriter(); using var error = new StringWriter();
                Assert.Equal(1, Command.Run(["--trace", "-"], new StringReader(input), output, error));
                using var json = JsonDocument.Parse(output.ToString());
                Assert.Equal(2, json.RootElement.GetProperty("obligations")[0].GetProperty("deadline_sample").GetInt64()); break;
        }
    }
}
