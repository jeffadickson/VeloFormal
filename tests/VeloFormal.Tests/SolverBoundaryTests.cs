using System.Collections.Generic;
using System.Text.Json.Nodes;
using VeloFormal.Language;
using VeloFormal.Solving;
using Xunit;
namespace VeloFormal.Tests;
public sealed class SolverBoundaryTests
{
    [Theory]
    [InlineData(521, "boolean_state")]
    [InlineData(522, "runtime_integer")]
    [InlineData(523, "expression_budget")]
    [InlineData(524, "invalid_timeout")]
    [InlineData(525, "repeat")]
    public void SolverBoundaries(int iteration, string scenario)
    {
        Assert.InRange(iteration, 521, 525);
        if (scenario == "runtime_integer")
        {
            var binder = new SemanticBinder(SemanticTests.Model());
            Assert.Throws<LanguageException>(() => binder.BindExpression(LanguageExpressions.Parse("ite(enable,1,2) < 3"))); return;
        }
        var model = SemanticTests.Model();
        string contract = ContractParserTests.Minimal.Replace("safe: true", "safe: count == 0");
        if (scenario == "boolean_state")
        {
            model = TransitionModel.Parse("""{"module":"m","clock":"clk","signals":[{"name":"q","type":"bool","kind":"state"}],"init":"!q","next":{"q":"!q"}}""");
            contract = ContractParserTests.Minimal.Replace("safe: true", "safe: !q");
        }
        var bound = SemanticBinder.Bind(ContractParser.Parse(contract), model);
        var options = new SolverOptions(Depth: 1, MaxExpressions: scenario == "expression_budget" ? 1 : 200000,
            TimeoutMilliseconds: scenario == "invalid_timeout" ? 0u : 10000u);
        if (scenario == "invalid_timeout") { Assert.Throws<InputException>(() => BoundedChecker.Check(bound, options)); return; }
        var report = BoundedChecker.Check(bound, options);
        Assert.Equal(scenario == "expression_budget" ? "unknown" : "counterexample", report.Status);
        if (scenario == "repeat")
        {
            Assert.Equal(report.Status, BoundedChecker.Check(bound, options).Status);
            Assert.Throws<NotSupportedException>(() => ((IDictionary<string,string>)report.Counterexample[0].Values).Clear());
        }
    }
}
