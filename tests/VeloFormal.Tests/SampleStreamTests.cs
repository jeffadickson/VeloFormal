using Xunit;
namespace VeloFormal.Tests;
public sealed class SampleStreamTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["empty", "blank", "one", "two", "crlf", "no_newline", "negative", "boolean", "big_integer",
            "float", "xz", "duplicate", "array", "malformed", "line_limit", "exact_limit", "invalid_limit", "line_number", "lazy", "reader_ownership"];
        return names.Take(20).Select((name, index) => new object[] { 261 + index, name });
    }
    [Theory, MemberData(nameof(Cases))]
    public void StreamBoundaries(int iteration, string scenario)
    {
        Assert.InRange(iteration, 261, 280);
        string text = scenario switch
        {
            "empty" => "", "blank" => " \n\t", "two" => "{}\n{}", "crlf" => "{}\r\n{}\r\n",
            "negative" => """{"n":-123}""", "boolean" => """{"ok":true}""",
            "big_integer" => """{"n":999999999999999999999999999999999999}""",
            "float" => """{"n":0.5}""", "xz" => """{"n":"X"}""",
            "duplicate" => """{"n":0,"n":1}""", "array" => "[]", "malformed" => "{",
            "line_number" => "\n{bad}", "lazy" => "{}\n{bad}", _ => "{}"
        };
        using var reader = new StringReader(text);
        int limit = scenario == "line_limit" ? 1 : scenario == "exact_limit" ? 2 : scenario == "invalid_limit" ? 0 : 1048576;
        var samples = SampleStream.ReadJsonLines(reader, limit);
        if (scenario is "float" or "xz" or "duplicate" or "array" or "malformed" or "line_limit" or "invalid_limit" or "line_number")
        {
            var error = Assert.Throws<InputException>(() => samples.ToArray());
            if (scenario == "line_number") Assert.Contains("line 2", error.Message);
        }
        else if (scenario == "lazy") Assert.Single(samples.Take(1));
        else
        {
            var result = samples.ToArray();
            Assert.Equal(scenario is "empty" or "blank" ? 0 : scenario is "two" or "crlf" ? 2 : 1, result.Length);
            Assert.Equal(-1, reader.Peek());
        }
    }
}
