using System.Globalization;
using System.Numerics;
using System.Text.Json.Nodes;
using VeloFormal.Language;
using VeloFormal.Solving;
using Xunit;
namespace VeloFormal.Tests;
public sealed class SolverCoreTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["initial_safe", "initial_fails_next", "range_fail", "wrap_safe", "model_unsat", "assumption_unsat",
            "always_false", "always_true", "future_requirement_unsat", "future_assumptions_do_not_hide", "signed_wrap",
            "unsigned_wrap", "sign_cast", "resize", "bitwise", "conditional_next", "input_conflict", "invalid_depth", "query_budget", "cancelled"];
        return names.Select((name, index) => new object[] { 501 + index, name });
    }
    public static Sample Sample(SolverFrame frame) => new(frame.Values.Select(p => new KeyValuePair<string, BigInteger>(
        p.Key, BigInteger.Parse(p.Value, CultureInfo.InvariantCulture))));
    public static void ReplayTransitions(SolverReport report, BoundContract bound)
    {
        for (int time = 1; time < report.Counterexample.Count; time++)
            foreach (var state in bound.Model.Signals.Where(s => s.IsState))
                Assert.Equal(TypedEvaluation.Evaluate(bound.Next[state.Name], Sample(report.Counterexample[time - 1])),
                    Sample(report.Counterexample[time])[state.Name]);
    }
    [Theory, MemberData(nameof(Cases))]
    public void NativeBoundedChecks(int iteration, string scenario)
    {
        Assert.InRange(iteration, 501, 520);
        var model = JsonNode.Parse(SemanticTests.ModelJson)!.AsObject();
        string invariant = "count == 0", requirement = "";
        int depth = 1; string expected = "counterexample";
        switch (scenario)
        {
            case "initial_safe": depth = 0; expected = "bounded-pass"; break;
            case "initial_fails_next": break;
            case "range_fail": invariant = "count <= 1"; depth = 2; break;
            case "wrap_safe": model["signals"]![0]!["width"] = 3; invariant = "count <= 7"; depth = 8; expected = "bounded-pass"; break;
            case "model_unsat": model["init"] = "false"; expected = "model-unsat"; break;
            case "assumption_unsat": requirement = "false"; expected = "assumption-unsat"; break;
            case "always_false": invariant = "false"; break;
            case "always_true": invariant = "true"; expected = "bounded-pass"; break;
            case "future_requirement_unsat": requirement = "count < 2"; invariant = "count < 2"; depth = 3; expected = "assumption-unsat"; break;
            case "future_assumptions_do_not_hide": requirement = "count < 2"; depth = 3; break;
            case "signed_wrap": model["signals"]![0]!["type"] = "signed"; model["init"] = "count == 127"; invariant = "count >= 0"; break;
            case "unsigned_wrap": model["init"] = "count == 255"; invariant = "count == 255"; break;
            case "sign_cast": model["init"] = "count == 255"; invariant = "signed(count) < 0"; break;
            case "resize": invariant = "resize(count,16) <= 16'd255"; depth = 8; expected = "bounded-pass"; break;
            case "bitwise": invariant = "(count & 8'd1) == count"; depth = 2; break;
            case "conditional_next": model["next"]!["count"] = "ite(enable, count + 1, count)"; invariant = "count < 2"; depth = 2; break;
            case "input_conflict": model["init"] = "count == 0 && enable"; requirement = "!enable"; expected = "assumption-unsat"; break;
            case "invalid_depth": depth = 129; break;
            case "query_budget": expected = "unknown"; break;
        }
        string source = "contract C for m\n    clock posedge clk\n" +
            (requirement == "" ? "" : "    require environment: " + requirement + "\n") +
            "    invariant safe: " + invariant + "\nend";
        var bound = SemanticBinder.Bind(ContractParser.Parse(source, "solver.vfc"), TransitionModel.Parse(model.ToJsonString()));
        var options = new SolverOptions(Depth: depth, MaxQueries: scenario == "query_budget" ? 1 : 4096);
        if (scenario == "invalid_depth") { Assert.Throws<InputException>(() => BoundedChecker.Check(bound, options)); return; }
        if (scenario == "cancelled")
        {
            using var cancel = new CancellationTokenSource(); cancel.Cancel();
            Assert.Throws<OperationCanceledException>(() => BoundedChecker.Check(bound, options, cancel.Token)); return;
        }
        var report = BoundedChecker.Check(bound, options);
        Assert.Equal(expected, report.Status); Assert.False(report.IsProof); Assert.StartsWith("Z3 ", report.Engine);
        if (expected == "counterexample")
        {
            Assert.NotEmpty(report.Counterexample); Assert.NotEmpty(report.Findings);
            Assert.Equal("solver.vfc", report.Findings[0].Span.File);
            ReplayTransitions(report, bound);
        }
        if (scenario == "future_assumptions_do_not_hide") Assert.Equal(1, report.Findings[0].Sample);
    }
}
