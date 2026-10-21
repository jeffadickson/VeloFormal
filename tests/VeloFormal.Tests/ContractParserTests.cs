using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class ContractParserTests
{
    public const string Minimal = "contract C for m\n    clock posedge clk\n    invariant safe: true\nend";
    public const string Operation = "contract C for m\n    clock posedge clk\n    operation write when valid && ready:\n        ensure rise after 1 cycles:\n            count == at_trigger(count) + 1\nend";
    public static IEnumerable<object[]> Cases()
    {
        string[] cases = [Minimal, Operation, "// lead\n" + Minimal,
            Minimal.Replace("\nend", ""), Minimal + "\nextra", Minimal.Replace("    clock posedge clk\n", ""),
            Minimal.Replace("    invariant", "    clock posedge clk\n    invariant"),
            Minimal.Replace("    invariant", "  invariant"),
            Minimal.Replace("invariant safe: true", "ensure safe after 1 cycles: true"),
            "contract C for m\n    clock posedge clk\n    operation x when true:\nend",
            Minimal.Replace("    invariant", "    parameter N: integer where N > 0\n    parameter N: integer where N > 0\n    invariant"),
            Minimal.Replace("\nend", "\n    cover safe: true\nend"),
            Minimal.Replace("posedge", "negedge"),
            Operation.Replace("\nend", "\n        ensure other within 1..3 cycles: true\nend"),
            Minimal.Replace("safe: true", "safe:\n        (true &&\n         true)")];
        return cases.Select((source, index) => new object[] { 426 + index, source, index is 0 or 1 or 2 or 13 or 14 });
    }
    [Theory, MemberData(nameof(Cases))]
    public void ParseContractStructure(int iteration, string source, bool valid)
    {
        Assert.InRange(iteration, 426, 440);
        if (!valid) { Assert.Throws<LanguageException>(() => ContractParser.Parse(source, "fixture.vfc")); return; }
        var syntax = ContractParser.Parse(source, "fixture.vfc");
        Assert.Equal("C", syntax.Name); Assert.Equal("m", syntax.Module); Assert.Equal("clk", syntax.Clock);
        Assert.Equal("fixture.vfc", syntax.Span.File);
    }
}
