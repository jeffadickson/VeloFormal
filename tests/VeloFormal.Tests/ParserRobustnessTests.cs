using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
/// <summary>Regression and malformed-input corpus for the indentation-sensitive contract parser.</summary>
public sealed class ParserRobustnessTests
{
    public static IEnumerable<object[]> Cases()
    {
        string full = "contract FifoSafety for fifo\n    parameter DEPTH: integer where DEPTH > 0\n    clock posedge clk\n    reset when !rst_n\n    require environment: !(rd_en && empty)\n    invariant count_range: unsigned(count) <= DEPTH\n    operation write when wr_en && !full:\n        ensure rises after 1 cycles:\n            count == at_trigger(count) + 1\n    cover becomes_full: full\nend";
        string[] sources = [full, full.Replace("\n", "\r\n"), "// comment\n" + full, full.Replace("count == at_trigger(count) + 1", "ite(full, count == at_trigger(count), count == at_trigger(count) + 1)"),
            full.Replace("    invariant", "      invariant"), full.Replace("        ensure", "      ensure"), full.Replace("            count", "          count"), full.Replace("end", "  end"),
            full.Replace("after 1 cycles", "after 1 cycle"), full.Replace("within", "with"), full.Replace("clock posedge clk", "clock posedge clk\n    clock posedge clk"),
            full.Replace("cover becomes_full: full", "cover becomes_full:\n        full"), full.Replace("parameter DEPTH: integer where DEPTH > 0", "parameter DEPTH integer where DEPTH > 0"),
            full.Replace("operation write when wr_en && !full:", "operation write when wr_en && !full"), full.Replace("end", "end\nend")];
        bool[] valid = [true, true, true, true, false, false, false, false, false, false, false, true, false, false, false];
        return sources.Select((source, index) => new object[] { 561 + index, source, valid[index] });
    }

    [Theory, MemberData(nameof(Cases))]
    public void PreservesStructureOrRejectsMalformedInput(int iteration, string source, bool valid)
    {
        Assert.InRange(iteration, 561, 575);
        if (!valid)
        {
            var error = Assert.Throws<LanguageException>(() => ContractParser.Parse(source, "robust.vfc"));
            Assert.Equal("robust.vfc", error.Diagnostic.Span.File);
            Assert.True(error.Diagnostic.Span.Line >= 1);
            return;
        }
        var syntax = ContractParser.Parse(source, "robust.vfc");
        Assert.Equal("FifoSafety", syntax.Name);
        Assert.Equal("fifo", syntax.Module);
        Assert.Equal("clk", syntax.Clock);
        Assert.Single(syntax.Operations);
        Assert.Single(syntax.Operations[0].Ensures);
    }
}
