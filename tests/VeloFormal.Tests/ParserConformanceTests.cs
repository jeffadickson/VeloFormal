using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ParserConformanceTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] source = [
            ContractParserTests.Minimal.Replace("safe: true", "safe:"),
            ContractParserTests.Minimal.Replace("safe: true", "safe true"),
            ContractParserTests.Operation.Replace("after 1", "until response"),
            ContractParserTests.Operation.Replace("after 1", "eventually"),
            ContractParserTests.Minimal.Replace("\nend", "\n    end"),
            ContractParserTests.Minimal.Replace("for m", "for top.fifo"),
            ContractParserTests.Minimal.Replace("    invariant", "    reset when !rst_n\n    invariant"),
            ContractParserTests.Minimal.Replace("    invariant", "    parameter DEPTH: integer where DEPTH > 0\n    invariant"),
            ContractParserTests.Operation.Replace("after 1", "within A..B"),
            ContractParserTests.Minimal.Replace("safe: true", "safe:\n        empty == (count == 0)")];
        return source.Select((text, index) => new object[] { 451 + index, text, index >= 5 });
    }
    [Theory, MemberData(nameof(Cases))]
    public void SurfaceConformance(int iteration, string source, bool valid)
    {
        Assert.InRange(iteration, 451, 460);
        if (valid) Assert.Equal("C", ContractParser.Parse(source).Name);
        else Assert.Throws<LanguageException>(() => ContractParser.Parse(source));
    }
}
