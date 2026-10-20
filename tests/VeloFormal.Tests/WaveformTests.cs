using Xunit;

namespace VeloFormal.Tests;

public sealed class WaveformTests
{
    private const string Header = """
        $scope module tb $end
        $var wire 1 ! clk $end
        $var wire 8 " count [7:0] $end
        $var wire 1 % unused $end
        $upscope $end
        $enddefinitions $end
        """;
    private const string Body = """
        #0
        $dumpvars
        0!
        b0 "
        x%
        $end
        #5
        1!
        b00000001 "
        #10
        0!
        #15
        1!
        b00000010 "
        #20
        0!
        """;
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["baseline", "alias", "uppercase", "comment", "unknown_unselected",
            "eof", "duplicate_time", "short_vector", "x", "z", "missing_clock", "missing_signal",
            "bus_clock", "backward", "unknown_id", "overflow", "none", "missing_header", "truncated", "glitch"];
        return names.Select((name, index) => new object[] { index + 151, name });
    }

    [Theory]
    [MemberData(nameof(Cases))]
    public void DecodeSettledRisingEdges(int iteration, string scenario)
    {
        Assert.InRange(iteration, 151, 170);
        string header = Header, body = Body, clock = "tb.clk";
        var signals = new Dictionary<string, string> { ["count"] = "tb.count" };
        switch (scenario)
        {
            case "alias": header = header.Replace("$upscope", "$var reg 8 \" alias $end\n$upscope"); signals["count"] = "tb.alias"; break;
            case "uppercase": body = body.Replace('b', 'B'); break;
            case "comment":
                header = header.Replace("wire 1 %", "parameter 1 %");
                body = body.Replace("#5", "$comment arbitrary text $end\n#5\n$dumpall\n$end"); break;
            case "eof": body = body[..body.IndexOf("#20", StringComparison.Ordinal)]; break;
            case "duplicate_time": body = body.Replace("#5", "#5\n#5"); break;
            case "short_vector": body = body.Replace("b00000001", "b1").Replace("b00000010", "b10"); break;
            case "x": body = body.Replace("b00000001", "bx"); break;
            case "z": body = body.Replace("b00000001", "bz"); break;
            case "missing_clock": clock = "tb.no_clock"; break;
            case "missing_signal": signals["count"] = "tb.missing"; break;
            case "bus_clock": header = header.Replace("wire 1 !", "wire 2 !"); break;
            case "backward": body = body.Replace("#10", "#4"); break;
            case "unknown_id": body = body.Replace("x%", "xINVALID"); break;
            case "overflow": body = body.Replace("b00000001", "b100000000"); break;
            case "none": signals.Clear(); break;
            case "missing_header": header = ""; break;
            case "truncated": body += "\nb1"; break;
            case "glitch": body = body.Replace("1!", "1! 0! 1!"); break;
        }
        if (iteration >= 159)
        {
            Assert.Throws<InputException>(() => VcdReader.Read(header + "\n" + body, clock, signals));
            return;
        }
        var samples = VcdReader.Read(header + "\n" + body, clock, signals);
        Assert.Equal(2, samples.Count);
        Assert.Equal(1, (int)samples[0]["count"]);
        Assert.Equal(2, (int)samples[1]["count"]);
    }
}
