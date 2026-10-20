using Xunit;
namespace VeloFormal.Tests;
public sealed class WaveformCliOptionTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[][] cases = [
            ["--vcd", "-", "--clock", "tb.clk", "--signal", "data=tb.data"],
            ["--vcd", "-", "--contract", "unused", "--signal", "data=tb.data"],
            ["--vcd", "-"],
            ["--vcd", "-", "--contract", "unused", "--clock", "tb.clk"],
            ["--vcd", "-", "--unexpected", "x"],
            ["--vcd", "-", "--clock", "x", "--clock", "y"],
            ["--vcd", "-", "--signal", "a=x", "--signal", "a=y"],
            ["--vcd", "-", "--signal", "broken"],
            ["--vcd", "-", "--clock"],
            ["--vcd", "-", "--signal", "True=x"]
        ];
        return cases.Select((args, index) => new object[] { 241 + index, args });
    }
    [Theory, MemberData(nameof(Cases))]
    public void InvalidOptions(int iteration, string[] args)
    {
        Assert.InRange(iteration, 241, 250);
        using var output = new StringWriter(); using var error = new StringWriter();
        Assert.Equal(2, Command.Run(args, TextReader.Null, output, error));
        Assert.Empty(output.ToString());
        Assert.StartsWith("veloformal:", error.ToString());
    }
}
