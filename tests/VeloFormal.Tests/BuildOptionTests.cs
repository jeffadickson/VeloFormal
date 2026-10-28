using VeloFormal.Build;
using System.Text.Json;
using Xunit;
namespace VeloFormal.Tests;
public sealed class BuildOptionTests
{
    [Fact]
    public void YosysProbeWritesVersionedDiagnosticArtifact()
    {
        string root = Path.Combine(Path.GetTempPath(), "veloformal-yosys-probe-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            using var output = new StringWriter();
            using var error = new StringWriter();

            Assert.Equal(0, YosysProbe.Run(root, output, error));
            string artifact = Directory.GetFiles(Path.Combine(root, "artifacts", "yosys-probe"), "toolchain.json", SearchOption.AllDirectories).Single();
            using var document = JsonDocument.Parse(File.ReadAllText(artifact));
            Assert.Equal("veloformal.yosys-probe.v1", document.RootElement.GetProperty("format").GetString());
            Assert.Equal(4, document.RootElement.GetProperty("tools").GetArrayLength());
            Assert.Contains("Yosys differential toolchain:", output.ToString(), StringComparison.Ordinal);
        }
        finally
        {
            if (Directory.Exists(root)) Directory.Delete(root, recursive: true);
        }
    }

    [Fact]
    public void ParsesYosysProbeModeAndRejectsBuildFlags()
    {
        Assert.Equal("yosys-probe", BuildOptions.Parse(["yosys-probe"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-probe", "--no-build"]));
        Assert.Equal("yosys-stage1", BuildOptions.Parse(["yosys-stage1"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-stage1", "--no-build"]));
        Assert.Equal("yosys-stage1-positive", BuildOptions.Parse(["yosys-stage1-positive"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-stage1-positive", "--no-build"]));
        Assert.Equal("yosys-stage1-deterministic", BuildOptions.Parse(["yosys-stage1-deterministic"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-stage1-deterministic", "--no-build"]));
        Assert.Equal("yosys-stage1-signed", BuildOptions.Parse(["yosys-stage1-signed"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-stage1-signed", "--no-build"]));
        Assert.Equal("yosys-stage1-concurrent", BuildOptions.Parse(["yosys-stage1-concurrent"]).Mode);
        Assert.Throws<InputException>(() => BuildOptions.Parse(["yosys-stage1-concurrent", "--no-build"]));
    }

    [Fact]
    public void YosysStage1RejectsMissingFixtureRoot()
    {
        string root = Path.Combine(Path.GetTempPath(), "veloformal-yosys-stage1-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            using var output = new StringWriter();
            using var error = new StringWriter();
            Assert.Equal(2, YosysStage1.Run(root, output, error));
            Assert.Contains("fixture files are missing", error.ToString(), StringComparison.Ordinal);
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, recursive: true); }
    }

    public static IEnumerable<object[]> Cases()
    {
        string[][] args = [[], ["test"], ["coverage"], ["rtl"], ["--help"], ["-h"],
            ["test", "--no-restore"], ["coverage", "--no-build"], ["--no-build", "coverage"],
            ["coverage", "--no-build", "--no-restore"], ["unknown"], ["test", "test"],
            ["--no-build", "--no-build"], ["--no-restore", "--no-restore"], ["--help", "test"],
            ["Coverage"], ["--unknown"], ["--no-build"], ["--no-restore"], ["rtl", "--no-build"]];
        return args.Take(20).Select((arguments, index) => new object[] { 361 + index, arguments });
    }
    [Theory, MemberData(nameof(Cases))]
    public void ParseBuildModes(int iteration, string[] args)
    {
        Assert.InRange(iteration, 361, 380);
        if (iteration is >= 371 and <= 377 or 380)
        {
            Assert.Throws<InputException>(() => BuildOptions.Parse(args)); return;
        }
        var options = BuildOptions.Parse(args);
        Assert.Equal(args.Contains("coverage") ? "coverage" : args.Contains("rtl") ? "rtl"
            : args.Contains("--help") || args.Contains("-h") ? "help" : "test", options.Mode);
        Assert.Equal(args.Contains("--no-build"), options.NoBuild);
        Assert.Equal(args.Contains("--no-restore"), options.NoRestore);
    }
}
