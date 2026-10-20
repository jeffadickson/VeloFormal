using VeloFormal.Build;
using Xunit;
namespace VeloFormal.Tests;
public sealed class BuildOptionTests
{
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
