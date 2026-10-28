using System.Text.Json;
using Xunit;

namespace VeloFormal.Tests;

public sealed class CliSchemaTests
{
    [Fact]
    public void PreservesRepeatableWaveformSignalMappings()
    {
        const int iteration = 626;
        Assert.InRange(iteration, 601, 650);
        string directory = Path.Combine(Path.GetTempPath(), "veloformal-cli-schema-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string contractPath = Path.Combine(directory, "contract.json");
            File.WriteAllText(contractPath, """{"contract":{"invariants":{"matches":"data == 2"}},"samples":[]}""");
            string[] args = ["--vcd", "-", "--contract", contractPath, "--clock", "tb.clk",
                "--signal", "data=tb.data", "--signal", "clock=tb.clk"];
            using var output = new StringWriter();
            using var error = new StringWriter();
            int exitCode = Command.Run(args, new StringReader(WaveformData.Header + WaveformData.Body), output, error);
            Assert.Equal(0, exitCode);
            using var report = JsonDocument.Parse(output.ToString());
            Assert.Equal("trace-pass", report.RootElement.GetProperty("status").GetString());
            Assert.Empty(error.ToString());
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }
}
