using Xunit;
namespace VeloFormal.Tests;
public sealed class VcdTimingTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["z_clock", "repeat_clock", "initial_x", "negative_descending", "negative_ascending",
            "cross_zero", "huge_indices", "huge_width_mismatch", "negative_time", "repeated_time",
            "offedge_unknown", "two_frames", "initial_high", "binary_clock", "missing_value"];
        return names.Select((name, index) => new object[] { 226 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void ClockAndRangeBoundaries(int iteration, string scenario)
    {
        Assert.InRange(iteration, 226, 240);
        string header = WaveformData.Header, body = WaveformData.Body;
        switch (scenario)
        {
            case "z_clock": body = body.Replace("1!", "z!"); break;
            case "repeat_clock": body = body.Replace("1!", "1! 1!"); break;
            case "initial_x": body = body.Replace("0!", "x!"); break;
            case "negative_descending": header = header.Replace("[7:0]", "[-1:-8]"); break;
            case "negative_ascending": header = header.Replace("[7:0]", "[-8:-1]"); break;
            case "cross_zero": header = header.Replace("[7:0]", "[-4:3]"); break;
            case "huge_indices": header = header.Replace("[7:0]", "[99999999999999999999999999999999999999999999999997:99999999999999999999999999999999999999999999999990]"); break;
            case "huge_width_mismatch": header = header.Replace("[7:0]", "[-99999999999999999999999:99999999999999999999999]"); break;
            case "negative_time": body = body.Replace("#5", "#-1"); break;
            case "repeated_time": body = body.Replace("#5", "#5 #5"); break;
            case "offedge_unknown": body = body.Replace("b0 %", "bx %"); break;
            case "two_frames": body += "#15 1! b11 % #20 0!"; break;
            case "initial_high": body = "#0 1! b0 % #5 1! b10 %"; break;
            case "binary_clock": body = body.Replace("0!", "b0 !").Replace("1!", "b1 !"); break;
            case "missing_value": body = "#0 0! #5 1!"; break;
        }
        if (scenario is "z_clock" or "initial_x" or "huge_width_mismatch" or "negative_time" or "missing_value")
            Assert.Throws<InputException>(() => VcdReader.ReadFrames(header + body, "tb.clk", WaveformData.Signals));
        else
        {
            var frames = VcdReader.ReadFrames(header + body, "tb.clk", WaveformData.Signals);
            Assert.Equal(scenario == "two_frames" ? 2 : scenario == "initial_high" ? 0 : 1, frames.Count);
            if (frames.Count > 0) Assert.Equal(5, frames[0].Timestamp);
            if (frames.Count == 2) Assert.Equal(15, frames[1].Timestamp);
        }
    }
}
