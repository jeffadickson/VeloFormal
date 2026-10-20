using System.Numerics;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ResourceTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["samples_zero", "samples_negative", "pending_zero", "pending_negative", "findings_zero", "findings_negative",
            "sample_limit", "reset_counts", "sample_exact", "pending_limit", "pending_exact", "reset_releases",
            "finding_limit", "finding_exact", "cancel_empty", "cancel_before", "cancel_midstream", "cancel_trigger", "assumption_stop", "null_samples"];
        return names.Select((name, index) => new object[] { 281 + index, name });
    }
    private static Sample S() => new(new Dictionary<string, BigInteger>());
    [Theory, MemberData(nameof(Cases))]
    public void BoundResources(int iteration, string scenario)
    {
        Assert.InRange(iteration, 281, 300);
        var valid = new Contract();
        var ensure = new Contract(ensures: [new("wait", _ => true, (_, _) => false, 5, 5)]);
        var bad = new Contract(invariants: [new("bad", _ => false)]);
        if (iteration <= 286)
        {
            var limits = scenario switch
            {
                "samples_zero" => new CheckOptions(MaxSamples: 0), "samples_negative" => new CheckOptions(MaxSamples: -1),
                "pending_zero" => new CheckOptions(MaxPending: 0), "pending_negative" => new CheckOptions(MaxPending: -1),
                "findings_zero" => new CheckOptions(MaxFindings: 0), _ => new CheckOptions(MaxFindings: -1)
            };
            Assert.Throws<InputException>(() => TraceChecker.Check(valid, [], limits));
            return;
        }
        using var cancellation = new CancellationTokenSource();
        switch (scenario)
        {
            case "sample_limit": Assert.Throws<InputException>(() => TraceChecker.Check(valid, [S(), S()], new(MaxSamples: 1))); break;
            case "reset_counts": Assert.Throws<InputException>(() => TraceChecker.Check(new(reset: _ => true), [S(), S()], new(MaxSamples: 1))); break;
            case "sample_exact": Assert.Equal("trace-pass", TraceChecker.Check(valid, [S()], new(MaxSamples: 1)).Status); break;
            case "pending_limit": Assert.Throws<InputException>(() => TraceChecker.Check(ensure, [S(), S()], new(MaxPending: 1))); break;
            case "pending_exact": Assert.Equal(1, TraceChecker.Check(ensure, [S()], new(MaxPending: 1)).Pending); break;
            case "reset_releases":
                int calls = 0;
                var resetting = new Contract(ensures: ensure.Ensures, reset: _ => ++calls == 2);
                Assert.Equal(1, TraceChecker.Check(resetting, [S(), S(), S()], new(MaxPending: 1)).Pending); break;
            case "finding_limit": Assert.Throws<InputException>(() => TraceChecker.Check(bad, [S(), S()], new(MaxFindings: 1))); break;
            case "finding_exact": Assert.Single(TraceChecker.Check(bad, [S()], new(MaxFindings: 1)).Findings); break;
            case "cancel_empty": cancellation.Cancel(); Assert.Throws<OperationCanceledException>(() => TraceChecker.Check(valid, [], cancellationToken: cancellation.Token)); break;
            case "cancel_before": cancellation.Cancel(); Assert.Throws<OperationCanceledException>(() => TraceChecker.Check(valid, [S()], cancellationToken: cancellation.Token)); break;
            case "cancel_midstream":
                IEnumerable<Sample> Samples() { yield return S(); cancellation.Cancel(); yield return S(); }
                Assert.Throws<OperationCanceledException>(() => TraceChecker.Check(valid, Samples(), cancellationToken: cancellation.Token)); break;
            case "cancel_trigger":
                var cancelling = new Contract(ensures: [new("cancel", _ => { cancellation.Cancel(); return true; }, (_, _) => true)]);
                Assert.Throws<OperationCanceledException>(() => TraceChecker.Check(cancelling, [S()], cancellationToken: cancellation.Token)); break;
            case "assumption_stop":
                var invalid = new Contract(requires: [new("legal", _ => false)]);
                Assert.Equal("invalid-assumptions", TraceChecker.Check(invalid, [S(), S()], new(MaxSamples: 1)).Status); break;
            case "null_samples": Assert.Throws<ArgumentNullException>(() => TraceChecker.Check(valid, null!)); break;
        }
    }
    [Theory]
    [InlineData("--samples", "", "--contract", "unused")]
    [InlineData("--samples", "-", "--contract", "")]
    [InlineData("--trace", "")]
    [InlineData("--trace", "\0")]
    public void InvalidPathsHaveDiagnostics(params string[] args)
    {
        using var output = new StringWriter(); using var error = new StringWriter();
        Assert.Equal(2, Command.Run(args, TextReader.Null, output, error));
        Assert.Empty(output.ToString());
        Assert.StartsWith("veloformal:", error.ToString());
    }
}
