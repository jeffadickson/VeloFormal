using System.Text.Json;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>End-to-end command-line coverage for the v0.2 parser and bounded checker.</summary>
public sealed class ContractCliTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["parse", "parse_multiline", "check_pass", "check_counterexample", "check_unknown",
            "bad_contract", "missing_model", "invalid_depth", "unknown_option", "clock_mismatch", "check_velohdl", "invalid_velohdl"];
        return names.Select((name, index) => new object[] { 551 + index, name });
    }

    [Theory, MemberData(nameof(Cases))]
    public void ContractCommandsReportMachineReadableResults(int iteration, string scenario)
    {
        Assert.InRange(iteration, 551, 562);
        string directory = Path.Combine(Path.GetTempPath(), "veloformal-cli-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            string contractPath = Path.Combine(directory, "check.vfc");
            string modelPath = Path.Combine(directory, "model.json");
            string contract = "contract C for m\n    clock posedge clk\n    invariant safe: count < 3\nend";
            if (scenario == "parse_multiline") contract = "contract C for m\n    clock posedge clk\n    invariant safe:\n        count < 2\nend";
            if (scenario == "bad_contract") contract = "contract C for m\n    clock negedge clk\nend";
            if (scenario == "check_counterexample") contract = "contract C for m\n    clock posedge clk\n    invariant safe: count == 0\nend";
            if (scenario == "clock_mismatch") contract = "contract C for m\n    clock posedge other\n    invariant safe: true\nend";
            File.WriteAllText(contractPath, contract);
            string model = SemanticTests.ModelJson;
            if (scenario == "check_velohdl") model = "{\"format\":\"velohdl.structural.v1\",\"provenance\":{\"source_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",\"frontend\":\"VeloSim\",\"frontend_revision\":\"pending\",\"normalized_ir_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",\"adapter\":\"VeloHdl.VeloSimNormalizedTranslator\",\"adapter_version\":\"1\"},\"model\":" + SemanticTests.ModelJson + "}";
            if (scenario == "invalid_velohdl") model = "{\"format\":\"velohdl.structural.v1\",\"provenance\":{\"source_sha256\":\"bad\",\"frontend\":\"VeloSim\",\"frontend_revision\":\"pending\",\"normalized_ir_sha256\":\"bad\",\"adapter\":\"adapter\",\"adapter_version\":\"1\"},\"model\":" + SemanticTests.ModelJson + "}";
            File.WriteAllText(modelPath, model);
            string[] args = scenario switch
            {
                "parse" or "parse_multiline" or "bad_contract" => ["--parse-contract", contractPath],
                "missing_model" => ["--check-contract", contractPath],
                "invalid_depth" => ["--check-contract", contractPath, "--model", modelPath, "--depth", "129"],
                "unknown_option" => ["--check-contract", contractPath, "--model", modelPath, "--wat"],
                "check_unknown" => ["--check-contract", contractPath, "--model", modelPath, "--depth", "2", "--max-queries", "1"],
                _ => ["--check-contract", contractPath, "--model", modelPath, "--depth", "2"]
            };
            using var output = new StringWriter();
            using var error = new StringWriter();
            int result = Command.Run(args, new StringReader(string.Empty), output, error);
            if (scenario is "bad_contract" or "missing_model" or "invalid_depth" or "unknown_option" or "clock_mismatch" or "invalid_velohdl")
            {
                Assert.Equal(2, result); Assert.Empty(output.ToString()); Assert.StartsWith("veloformal:", error.ToString()); return;
            }
            Assert.Empty(error.ToString());
            using var report = JsonDocument.Parse(output.ToString());
            if (scenario is "parse" or "parse_multiline")
            {
                Assert.Equal(0, result); Assert.Equal("parsed", report.RootElement.GetProperty("status").GetString());
                Assert.Equal("C", report.RootElement.GetProperty("contract").GetString()); return;
            }
            Assert.Equal(scenario is "check_pass" or "check_velohdl" ? 0 : scenario == "check_counterexample" ? 1 : 3, result);
            Assert.Equal(scenario is "check_pass" or "check_velohdl" ? "bounded-pass" : scenario == "check_counterexample" ? "counterexample" : "unknown", report.RootElement.GetProperty("status").GetString());
            if (scenario == "check_velohdl") Assert.Equal("VeloSim", report.RootElement.GetProperty("input_provenance").GetProperty("frontend").GetString());
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }
}
