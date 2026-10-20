using Xunit;
namespace VeloFormal.Tests;

public static class WaveformData
{
    public const string Header = "$scope module tb $end\n$var wire 1 ! clk $end\n$var wire 8 % data [7:0] $end\n$upscope $end\n$enddefinitions $end\n";
    public const string Body = "#0\n0!\nb0 %\n#5\n1!\nb10 %\n#10\n0!\n";
    public static readonly IReadOnlyDictionary<string, string> Signals = new Dictionary<string, string> { ["data"] = "tb.data" };
}
public sealed class VcdHeaderTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["scope_unclosed", "scope_underflow", "bad_range", "range_width", "extra_range", "zero_width",
            "negative_width", "huge_width", "duplicate_path", "alias_width",
            "ascending_range", "no_range", "reg", "logic", "integer", "parameter", "metadata", "alias", "scalar_vector_error", "single_bit_range"];
        return names.Select((name, index) => new object[] { 201 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void HeaderConformance(int iteration, string scenario)
    {
        string header = WaveformData.Header, body = WaveformData.Body;
        bool invalid = iteration <= 210 || scenario == "scalar_vector_error";
        switch (scenario)
        {
            case "scope_unclosed": header = header.Replace("$upscope $end", ""); break;
            case "scope_underflow": header = "$upscope $end\n" + header; break;
            case "bad_range": header = header.Replace("[7:0]", "[oops]"); break;
            case "range_width": header = header.Replace("[7:0]", "[8:0]"); break;
            case "extra_range": header = header.Replace("[7:0]", "[7:0] extra"); break;
            case "zero_width": header = header.Replace("wire 8", "wire 0"); break;
            case "negative_width": header = header.Replace("wire 8", "wire -1"); break;
            case "huge_width": header = header.Replace("wire 8", "wire 65537"); break;
            case "duplicate_path": header = header.Replace("$upscope", "$var reg 8 + data [7:0] $end\n$upscope"); break;
            case "alias_width": header = header.Replace("$upscope", "$var reg 7 % other [6:0] $end\n$upscope"); break;
            case "ascending_range": header = header.Replace("[7:0]", "[0:7]"); break;
            case "no_range": header = header.Replace("[7:0]", ""); break;
            case "reg": header = header.Replace("wire 8", "reg 8"); break;
            case "logic": header = header.Replace("wire 8", "logic 8"); break;
            case "integer": header = header.Replace("wire 8", "integer 8"); break;
            case "parameter": header = header.Replace("wire 8", "parameter 8"); break;
            case "metadata": header = "$date sample $end\n$timescale 1 ns $end\n$version test $end\n" + header; break;
            case "alias": header = header.Replace("$upscope", "$var reg 8 % alias [7:0] $end\n$upscope"); break;
            case "scalar_vector_error": body = body.Replace("b10 %", "1%"); break;
            case "single_bit_range": header = header.Replace("wire 1 ! clk", "wire 1 ! clk [0]"); break;
        }
        if (invalid) Assert.Throws<InputException>(() => VcdReader.Read(header + body, "tb.clk", WaveformData.Signals));
        else Assert.Equal(2, (int)Assert.Single(VcdReader.Read(header + body, "tb.clk", WaveformData.Signals))["data"]);
    }
}
