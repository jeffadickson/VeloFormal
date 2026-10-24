using System.Text.Json.Nodes;
using VeloFormal.Language;
using VeloFormal.Solving;
using Xunit;
namespace VeloFormal.Tests;
public sealed class TemporalSolverTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["exact_pass", "exact_fail", "window_lower", "window_upper", "window_late", "overlap", "independent",
            "reset_cancel", "reset_no_spawn", "pending", "satisfied_before_horizon", "zero_depth", "unreachable", "inclusive", "zero_delay"];
        return names.Select((name, index) => new object[] { 526 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void MonitorSymbolicObligations(int iteration, string scenario)
    {
        Assert.InRange(iteration, 526, 540);
        var model = JsonNode.Parse(SemanticTests.ModelJson)!.AsObject();
        string trigger = "count == 0", response = "count == at_trigger(count) + 1", delay = "after 1", reset = "";
        int depth = 4; string expected = "bounded-pass";
        switch (scenario)
        {
            case "exact_fail": model["next"]!["count"] = "count + 2"; expected = "counterexample"; break;
            case "window_lower": delay = "within 1..2"; break;
            case "window_upper": delay = "within 1..2"; response = "count == at_trigger(count) + 2"; break;
            case "window_late": delay = "within 1..2"; response = "count == at_trigger(count) + 3"; expected = "counterexample"; break;
            case "overlap": trigger = "count < 2"; delay = "after 2"; response = "count == at_trigger(count) + 2"; break;
            case "independent": trigger = "count < 2"; delay = "after 2"; response = "ite(at_trigger(count) == 0, count == 2, count == 99)"; expected = "counterexample"; break;
            case "reset_cancel":
            case "reset_no_spawn":
                model["signals"]!.AsArray().Add(new JsonObject { ["name"] = "running", ["type"] = "bool", ["kind"] = "state" });
                model["init"] = scenario == "reset_cancel" ? "count == 0 && running" : "count == 0 && !running";
                model["next"]!["running"] = "false"; model["next"]!["count"] = "count + 2";
                reset = "    reset when !running\n"; trigger = "true"; response = "false"; break;
            case "pending": delay = "after 5"; depth = 2; expected = "incomplete"; break;
            case "satisfied_before_horizon": delay = "within 1..5"; depth = 2; break;
            case "zero_depth": depth = 0; expected = "incomplete"; break;
            case "unreachable": trigger = "count == 200"; break;
            case "inclusive": delay = "within 2..2"; response = "count == at_trigger(count) + 2"; break;
            case "zero_delay": delay = "after 0"; break;
        }
        string source = $"contract C for m\n    clock posedge clk\n{reset}    operation write when {trigger}:\n        ensure response {delay} cycles:\n            {response}\nend";
        if (scenario == "zero_delay")
        {
            Assert.Throws<LanguageException>(() => SemanticBinder.Bind(ContractParser.Parse(source), TransitionModel.Parse(model.ToJsonString()))); return;
        }
        var bound = SemanticBinder.Bind(ContractParser.Parse(source, "temporal.vfc"), TransitionModel.Parse(model.ToJsonString()));
        var report = BoundedChecker.Check(bound, new(Depth: depth));
        Assert.Equal(expected, report.Status); Assert.False(report.IsProof);
        if (expected == "counterexample")
        {
            SolverCoreTests.ReplayTransitions(report, bound);
            Assert.Equal("response", report.Findings[0].Label);
            if (scenario == "independent") Assert.Equal(1, report.Findings[0].TriggerSample);
        }
        if (expected == "incomplete") Assert.Contains("response", report.PendingLabels);
        if (scenario == "unreachable") Assert.Equal("not-reached-within-checked-prefix", Assert.Single(report.Triggers).Status);
    }
}
