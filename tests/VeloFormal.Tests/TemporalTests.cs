using System.Numerics;
using Xunit;

namespace VeloFormal.Tests;

public sealed class TemporalTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["exact", "late", "early_only", "unfinished", "reset_deadline",
            "window_upper", "window_lower", "overlap", "independent_failure", "assumption_failure"];
        int iteration = 51;
        for (int delay = 1; delay <= 5; delay++)
            foreach (var name in names) yield return [iteration++, delay, name];
    }

    private static Sample S(bool trigger = false, bool done = false, int count = 0,
        bool reset = false, bool legal = true) => new(new Dictionary<string, BigInteger>
        {
            ["trigger"] = trigger ? 1 : 0, ["done"] = done ? 1 : 0, ["count"] = count,
            ["reset"] = reset ? 1 : 0, ["legal"] = legal ? 1 : 0
        });

    [Theory]
    [MemberData(nameof(Cases))]
    public void MonitorBoundaries(int iteration, int delay, string scenario)
    {
        Assert.InRange(iteration, 51, 100);
        var samples = Enumerable.Range(0, delay + 2).Select(_ => S()).ToList();
        samples[0] = S(trigger: true);
        int earliest = delay, latest = delay;
        string status = "trace-pass";
        bool captures = false;
        switch (scenario)
        {
            case "exact": samples[delay] = S(done: true); break;
            case "late": samples[delay + 1] = S(done: true); status = "counterexample"; break;
            case "early_only": samples[0] = S(trigger: true, done: true); status = "counterexample"; break;
            case "unfinished": samples = samples.Take(delay).ToList(); status = "incomplete"; break;
            case "reset_deadline": samples[delay] = S(reset: true); break;
            case "window_upper": earliest = 1; samples[delay] = S(done: true); break;
            case "window_lower": earliest = 1; samples[1] = S(done: true); break;
            case "overlap":
                earliest = 1; latest = delay + 1; captures = true;
                samples = [S(trigger: true, count: 0), S(trigger: true, count: 1), S(count: 2)];
                break;
            case "independent_failure":
                earliest = 1; latest = delay + 1; captures = true; status = "counterexample";
                samples = Enumerable.Range(0, latest + 2).Select(_ => S()).ToList();
                samples[0] = S(trigger: true); samples[1] = S(trigger: true);
                samples[latest + 1] = S(count: 1);
                break;
            case "assumption_failure": samples[delay] = S(legal: false); status = "invalid-assumptions"; break;
        }
        var contract = new Contract(
            requires: [new("legal_input", s => s["legal"] != 0)],
            invariants: [new("count_nonnegative", s => s["count"] >= 0)],
            ensures: [new("response", s => s["trigger"] != 0,
                (s, t) => captures ? s["count"] == t["count"] + 1 : s["done"] != 0, earliest, latest)],
            covers: [new("done_seen", s => s["done"] != 0)], reset: s => s["reset"] != 0);
        var report = TraceChecker.Check(contract, samples);
        Assert.Equal(status, report.Status);
        Assert.Empty(report.Untriggered);
        if (scenario == "reset_deadline") Assert.Equal(1, report.Cancelled);
        else Assert.Equal(0, report.Cancelled);
        Assert.Equal(scenario == "unfinished" ? 1 : scenario == "assumption_failure" ? 1 : 0, report.Pending);
        if (status == "counterexample")
        {
            var finding = Assert.Single(report.Findings);
            Assert.Equal("response", finding.Label);
            Assert.Equal(scenario == "independent_failure" ? delay + 1 : delay, finding.Sample);
            Assert.Equal(0, finding.TriggerSample);
        }
        if (scenario == "assumption_failure") Assert.Equal("require", Assert.Single(report.Findings).Kind);
    }
}
