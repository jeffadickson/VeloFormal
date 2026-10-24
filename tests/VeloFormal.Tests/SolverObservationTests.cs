using System.Text.Json.Nodes;
using VeloFormal.Language;
using VeloFormal.Solving;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>Exercises bounded cover/trigger observations and horizon reporting.</summary>
public sealed class SolverObservationTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["cover_reached", "cover_unreached", "cover_respects_require", "cover_reset", "pending_trigger",
            "partial_window_satisfied", "future_window_pending", "budget_unknown", "multiple_covers", "observations_immutable"];
        return names.Select((name, index) => new object[] { 541 + index, name });
    }

    [Theory, MemberData(nameof(Cases))]
    public void RecordsBoundedGoalObservations(int iteration, string scenario)
    {
        Assert.InRange(iteration, 541, 550);
        var model = JsonNode.Parse(SemanticTests.ModelJson)!.AsObject();
        var clauses = "    cover goal: count == 2\n";
        var operation = "";
        var depth = 3;
        var expected = "bounded-pass";
        switch (scenario)
        {
            case "cover_unreached": clauses = "    cover goal: count == 99\n"; break;
            case "cover_respects_require":
                clauses = "    require only_zero: count == 0\n    cover goal: count == 2\n"; expected = "assumption-unsat";
                break;
            case "cover_reset":
                model["signals"]!.AsArray().Add(new JsonObject { ["name"] = "running", ["type"] = "bool", ["kind"] = "state" });
                model["init"] = "count == 0 && !running";
                model["next"]!["running"] = "false";
                clauses = "    reset when !running\n    cover goal: count == 0\n";
                break;
            case "pending_trigger":
                operation = Ensure("count == 0", "after 5", "count == at_trigger(count) + 1"); depth = 2; expected = "incomplete";
                break;
            case "partial_window_satisfied":
                operation = Ensure("count == 0", "within 1..5", "count == at_trigger(count) + 1"); depth = 2;
                break;
            case "future_window_pending":
                operation = Ensure("count == 0", "within 2..5", "count == at_trigger(count) + 1"); depth = 2; expected = "incomplete";
                break;
            case "budget_unknown": depth = 2; break;
            case "multiple_covers": clauses = "    cover first: count == 1\n    cover second: count == 2\n"; break;
        }
        string source = $"contract C for m\n    clock posedge clk\n{clauses}{operation}end";
        var bound = SemanticBinder.Bind(ContractParser.Parse(source, "observe.vfc"), TransitionModel.Parse(model.ToJsonString()));
        var options = scenario == "budget_unknown" ? new SolverOptions(Depth: depth, MaxQueries: 1) : new SolverOptions(Depth: depth);
        var report = BoundedChecker.Check(bound, options);
        if (scenario == "budget_unknown") { Assert.Equal("unknown", report.Status); return; }
        Assert.Equal(expected, report.Status);
        Assert.False(report.IsProof);
        Assert.InRange(report.LastFeasibleSample, 0, depth);
        switch (scenario)
        {
            case "cover_reached": Assert.Equal("reached", Assert.Single(report.Covers).Status); break;
            case "cover_unreached": Assert.Equal("not-reached-within-checked-prefix", Assert.Single(report.Covers).Status); break;
            case "cover_respects_require": Assert.Equal("not-reached-within-checked-prefix", Assert.Single(report.Covers).Status); Assert.Equal(0, report.LastFeasibleSample); break;
            case "cover_reset": Assert.Equal("not-reached-within-checked-prefix", Assert.Single(report.Covers).Status); break;
            case "pending_trigger": Assert.Contains("response", report.PendingLabels); Assert.NotNull(report.Reason); break;
            case "partial_window_satisfied": Assert.Empty(report.PendingLabels); break;
            case "future_window_pending": Assert.Contains("response", report.PendingLabels); break;
            case "multiple_covers": Assert.All(report.Covers, c => Assert.Equal("reached", c.Status)); break;
            case "observations_immutable":
                Assert.IsAssignableFrom<IReadOnlyList<GoalObservation>>(report.Covers);
                Assert.Throws<NotSupportedException>(() => ((IList<GoalObservation>)report.Covers).Add(new("bad", "reached")));
                break;
        }
    }

    private static string Ensure(string trigger, string delay, string response) =>
        $"    operation write when {trigger}:\n        ensure response {delay} cycles:\n            {response}\n";
}
