using System.Text.Json;
using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>Exercises the checked-in formal example, including deliberate model mutations.</summary>
public sealed class ExampleWorkflowTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] cases = ["parse", "safe_depth", "bound_breach", "mutation_increment_two", "query_budget", "timeout_option", "duplicate_depth", "missing_value", "unknown_option", "model_clock", "model_json", "source_name", "source_module", "source_clock", "source_operation", "source_ensure", "source_cover", "source_span", "mutated_invariant", "ensure_counterexample"];
        return cases.Select((name, index) => new object[] { 581 + index, name });
    }

    [Theory, MemberData(nameof(Cases))]
    public void CheckedInExampleHasDefinedBoundedBehavior(int iteration, string scenario)
    {
        Assert.InRange(iteration, 581, 600);
        string contractPath = Path.Combine(AppContext.BaseDirectory, "examples", "counter.vfc");
        string modelPath = Path.Combine(AppContext.BaseDirectory, "examples", "counter.model");
        string contract = File.ReadAllText(contractPath);
        string model = File.ReadAllText(modelPath);
        if (scenario is "source_name" or "source_module" or "source_clock" or "source_operation" or "source_ensure" or "source_cover" or "source_span")
        {
            var syntax = ContractParser.Parse(contract, contractPath);
            Assert.Equal("CounterSafety", syntax.Name); Assert.Equal("counter", syntax.Module); Assert.Equal("clk", syntax.Clock);
            Assert.Single(syntax.Operations); Assert.Single(syntax.Operations[0].Ensures); Assert.Single(syntax.Clauses, c => c.Role == ClauseRole.Cover);
            Assert.Equal(contractPath, syntax.Span.File); return;
        }
        string directory = Path.Combine(Path.GetTempPath(), "veloformal-example-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            string localContract = Path.Combine(directory, "counter.vfc");
            string localModel = Path.Combine(directory, "counter.model");
            if (scenario is "mutation_increment_two" or "ensure_counterexample") model = model.Replace("count + 1", "count + 2");
            if (scenario == "bound_breach") contract = contract.Replace("count <= 8'd3", "count <= 8'd2");
            if (scenario == "mutated_invariant") contract = contract.Replace("count <= 8'd3", "count == 0");
            if (scenario == "model_clock") model = model.Replace("\"clk\"", "\"other\"");
            if (scenario == "model_json") model = "{";
            File.WriteAllText(localContract, contract); File.WriteAllText(localModel, model);
            string[] args = scenario switch
            {
                "parse" => ["--parse-contract", localContract],
                "safe_depth" or "timeout_option" => ["--check-contract", localContract, "--model", localModel, "--depth", "3", "--timeout-ms", "10000"],
                "bound_breach" => ["--check-contract", localContract, "--model", localModel, "--depth", "4"],
                "mutation_increment_two" or "mutated_invariant" or "ensure_counterexample" => ["--check-contract", localContract, "--model", localModel, "--depth", "2"],
                "query_budget" => ["--check-contract", localContract, "--model", localModel, "--max-queries", "1"],
                "duplicate_depth" => ["--check-contract", localContract, "--model", localModel, "--depth", "1", "--depth", "2"],
                "missing_value" => ["--check-contract", localContract, "--model"],
                "unknown_option" => ["--check-contract", localContract, "--model", localModel, "--unknown", "x"],
                _ => ["--check-contract", localContract, "--model", localModel, "--depth", "1"]
            };
            using var output = new StringWriter(); using var error = new StringWriter();
            int exit = Command.Run(args, TextReader.Null, output, error);
            if (scenario is "model_clock" or "model_json" or "duplicate_depth" or "missing_value" or "unknown_option") { Assert.Equal(2, exit); return; }
            using var json = JsonDocument.Parse(output.ToString());
            if (scenario == "parse") { Assert.Equal(0, exit); Assert.Equal("parsed", json.RootElement.GetProperty("status").GetString()); return; }
            string status = json.RootElement.GetProperty("status").GetString()!;
            if (scenario == "query_budget") { Assert.Equal(3, exit); Assert.Equal("unknown", status); return; }
            if (scenario is "bound_breach" or "mutation_increment_two" or "mutated_invariant" or "ensure_counterexample") { Assert.Equal(1, exit); Assert.Equal("counterexample", status); return; }
            if (scenario is "safe_depth" or "timeout_option") { Assert.Equal(1, exit); Assert.Equal("incomplete", status); return; }
            Assert.Equal(0, exit); Assert.Equal("bounded-pass", status);
        }
        finally { Directory.Delete(directory, recursive: true); }
    }
}
