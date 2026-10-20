using System.Text.Json;
using Xunit;
namespace VeloFormal.Tests;
public sealed class WaveformCliIntegrationTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["pass", "invariant_failure", "require_failure", "uninitialized_clock", "unknown_clock",
            "missing_signal", "nonempty_samples", "stdin", "duplicate_option", "space_paths"];
        return names.Select((name, index) => new object[] { 251 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void CheckWaveform(int iteration, string scenario)
    {
        Assert.InRange(iteration, 251, 260);
        string directory = Path.Combine(Path.GetTempPath(), "velo wave " + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string contract = Path.Combine(directory, "contract.trace"), waveform = Path.Combine(directory, "recording.vcd");
            string json = scenario == "require_failure"
                ? """{"contract":{"requires":{"legal":"False"}},"samples":[]}"""
                : """{"contract":{"invariants":{"matches":"data == 2"}},"samples":[]}""";
            if (scenario == "invariant_failure") json = json.Replace("data == 2", "data == 3");
            if (scenario == "nonempty_samples") json = json.Replace("\"samples\":[]", "\"samples\":[{}]");
            string body = scenario == "uninitialized_clock" ? "#0 b0 %" : WaveformData.Body;
            if (scenario == "unknown_clock") body = body.Replace("1!", "x!");
            File.WriteAllText(contract, json);
            File.WriteAllText(waveform, WaveformData.Header + body);
            var args = new List<string> { "--vcd", scenario == "stdin" ? "-" : waveform, "--contract", contract,
                "--clock", "tb.clk", "--signal", scenario == "missing_signal" ? "data=tb.absent" : "data=tb.data" };
            if (scenario == "duplicate_option") args.AddRange(["--contract", contract]);
            using var output = new StringWriter(); using var error = new StringWriter();
            int code = Command.Run(args.ToArray(), new StringReader(WaveformData.Header + body), output, error);
            int expected = scenario is "unknown_clock" or "missing_signal" or "nonempty_samples" or "duplicate_option" ? 2
                : scenario is "invariant_failure" or "require_failure" or "uninitialized_clock" ? 1 : 0;
            Assert.Equal(expected, code);
            if (expected == 2) Assert.Empty(output.ToString());
            else
            {
                using var report = JsonDocument.Parse(output.ToString());
                string status = report.RootElement.GetProperty("status").GetString()!;
                Assert.Equal(scenario == "invariant_failure" ? "counterexample" : scenario == "require_failure"
                    ? "invalid-assumptions" : scenario == "uninitialized_clock" ? "incomplete" : "trace-pass", status);
            }
        }
        finally { Directory.Delete(directory, recursive: true); }
    }
}
