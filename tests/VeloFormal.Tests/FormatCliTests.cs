using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>Verifies the canonical formatter command's file and argument handling.</summary>
public sealed class FormatCliTests
{
    public static IEnumerable<object[]> Cases() => new object[][]
    {
        [616, "valid", 0], [617, "missing", 2], [618, "duplicate", 2], [619, "extra", 2], [620, "invalid", 2]
    };
    [Theory, MemberData(nameof(Cases))]
    public void FormatsOnlyValidNamedContracts(int iteration, string scenario, int expected)
    {
        Assert.InRange(iteration, 616, 620);
        string path = Path.Combine(Path.GetTempPath(), "veloformal-format-" + Guid.NewGuid().ToString("N") + ".vfc");
        try
        {
            File.WriteAllText(path, scenario == "invalid" ? "contract C for m\nend" : ContractParserTests.Minimal);
            string[] args = scenario switch
            {
                "missing" => ["--format-contract"],
                "duplicate" => ["--format-contract", path, "--format-contract", path],
                "extra" => ["--format-contract", path, "--extra", "x"],
                _ => ["--format-contract", path]
            };
            using var output = new StringWriter(); using var error = new StringWriter();
            Assert.Equal(expected, Command.Run(args, TextReader.Null, output, error));
            if (expected == 0) { Assert.Equal(ContractFormatter.Format(ContractParser.Parse(ContractParserTests.Minimal, path)), output.ToString()); Assert.Empty(error.ToString()); }
            else { Assert.Empty(output.ToString()); Assert.StartsWith("veloformal:", error.ToString()); }
        }
        finally { File.Delete(path); }
    }
}
