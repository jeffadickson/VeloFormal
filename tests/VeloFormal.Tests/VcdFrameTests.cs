using Xunit;
namespace VeloFormal.Tests;
public sealed class VcdFrameTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["timestamp", "large_timestamp", "eof_sample", "constant_clock", "unknown_clock"];
        return names.Select((name, index) => new object[] { 221 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void FrameMetadata(int iteration, string scenario)
    {
        Assert.InRange(iteration, 221, 225);
        string body = WaveformData.Body;
        if (scenario == "large_timestamp") body = body.Replace("#5", "#5000000000").Replace("#10", "#6000000000");
        if (scenario == "eof_sample") body = body[..body.IndexOf("#10", StringComparison.Ordinal)];
        if (scenario == "constant_clock") body = body.Replace("1!", "0!");
        if (scenario == "unknown_clock") body = body.Replace("1!", "x!");
        if (scenario == "unknown_clock")
        {
            Assert.Throws<InputException>(() => VcdReader.ReadFrames(WaveformData.Header + body, "tb.clk", WaveformData.Signals));
            return;
        }
        var frames = VcdReader.ReadFrames(WaveformData.Header + body, "tb.clk", WaveformData.Signals);
        if (scenario == "constant_clock") Assert.Empty(frames);
        else
        {
            var frame = Assert.Single(frames);
            Assert.Equal(scenario == "large_timestamp" ? 5000000000L : 5L, frame.Timestamp);
            Assert.Equal(2, (int)frame.Sample["data"]);
        }
    }
}
