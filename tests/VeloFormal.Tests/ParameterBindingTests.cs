using System.Text.Json.Nodes;
using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ParameterBindingTests
{
    [Theory]
    [InlineData(476, "positive")]
    [InlineData(477, "zero")]
    [InlineData(478, "missing")]
    [InlineData(479, "delay_expression")]
    [InlineData(480, "shadow")]
    public void ElaborateStaticParameters(int iteration, string scenario)
    {
        Assert.InRange(iteration, 476, 480);
        var root = JsonNode.Parse(SemanticTests.ModelJson)!.AsObject();
        if (scenario != "missing") root["parameters"] = new JsonObject { [scenario == "shadow" ? "count" : "DEPTH"] = scenario == "zero" ? 0 : 2 };
        if (scenario == "shadow") { Assert.Throws<InputException>(() => TransitionModel.Parse(root.ToJsonString())); return; }
        var model = TransitionModel.Parse(root.ToJsonString());
        string source = "contract C for m\n    parameter DEPTH: integer where DEPTH > 0\n    clock posedge clk\n    operation write when enable:\n        ensure rise after DEPTH cycles:\n            count == at_trigger(count) + 1\nend";
        if (scenario == "delay_expression") source = source.Replace("after DEPTH", "after DEPTH + 1");
        if (scenario is "zero" or "missing") Assert.Throws<LanguageException>(() => SemanticBinder.Bind(ContractParser.Parse(source), model));
        else Assert.Equal(scenario == "delay_expression" ? 3 : 2, Assert.Single(SemanticBinder.Bind(ContractParser.Parse(source), model).Ensures).Latest);
    }
}
