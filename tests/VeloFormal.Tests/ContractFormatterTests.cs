using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>Checks that canonical formatting preserves the parser's meaningful contract structure.</summary>
public sealed class ContractFormatterTests
{
    public static IEnumerable<object[]> Cases()
    {
        string full = "contract F for m\n    parameter N: integer where N > 0\n    clock posedge clk\n    reset when !rst\n    require input: a || b && c\n    invariant safe: (count + 1) <= 8'd7\n    operation write when valid && ready:\n        ensure one after 1 cycles:\n            count == at_trigger(count) + 1\n        ensure window within 1..3 cycles:\n            count >= at_trigger(count)\n    cover seen: full\nend";
        string[] sources = [full, full.Replace("\n", "\r\n"), full.Replace("a || b && c", "(a || (b && c))"),
            full.Replace("a || b && c", "(a < b) == true"),
            full.Replace("count + 1", "(count + 1)"), full.Replace("valid && ready", "(valid && ready)"),
            full.Replace("count >= at_trigger(count)", "ite(full, count >= at_trigger(count), count >= at_trigger(count))"),
            full.Replace("    reset when !rst\n", ""), full.Replace("    parameter N: integer where N > 0\n", ""),
            full.Replace("    cover seen: full\n", ""), full.Replace("    require input: a || b && c\n", ""),
            full.Replace("    invariant safe: (count + 1) <= 8'd7\n", ""), full.Replace("ensure one after 1 cycles", "ensure one within 1..1 cycles"),
            full.Replace("write", "accept"), full.Replace("F for m", "G for n"), "contract C for m\n    clock posedge clk\n    invariant ok: true\nend"];
        return sources.Select((source, index) => new object[] { 601 + index, source });
    }

    [Fact]
    public void FormatsLongLeftAssociatedChainWithinParserDepth()
    {
        string expression = string.Join(" + ", Enumerable.Repeat("count", 40));
        string source = "contract C for m\n    clock posedge clk\n    invariant deep: " + expression + " == count\nend";
        string formatted = ContractFormatter.Format(ContractParser.Parse(source));
        Assert.Contains("count + count + count", formatted);
        Assert.NotNull(ContractParser.Parse(formatted));
    }

    [Theory, MemberData(nameof(Cases))]
    public void FormatRoundTripsStableSyntax(int iteration, string source)
    {
        Assert.InRange(iteration, 601, 616);
        var first = ContractParser.Parse(source, "input.vfc");
        string formatted = ContractFormatter.Format(first);
        var second = ContractParser.Parse(formatted, "formatted.vfc");
        Assert.Equal(first.Name, second.Name); Assert.Equal(first.Module, second.Module); Assert.Equal(first.Clock, second.Clock);
        Assert.Equal(first.Parameters.Count, second.Parameters.Count); Assert.Equal(first.Clauses.Count, second.Clauses.Count);
        Assert.Equal(first.Operations.Count, second.Operations.Count);
        Assert.Equal(formatted, ContractFormatter.Format(second));
    }
}
