using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class LanguageLexerTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] text = ["", "contract C for m", "\n", "\r\n", "\r", "// comment\nx", "/* c */x", "/* a\nb */x",
            "a && b || !c", "a==b != c <= d >= e", "1..3", "8'hff", "8'sd1", "_signal", "count_1",
            "\t", "@", "/*", "8'q1", "8'b"];
        return text.Select((source, index) => new object[] { 401 + index, source });
    }
    [Theory, MemberData(nameof(Cases))]
    public void LexWithLocations(int iteration, string source)
    {
        Assert.InRange(iteration, 401, 420);
        if (iteration >= 416)
        {
            var error = Assert.Throws<LanguageException>(() => Lexer.Tokenize(source, "case.vfc"));
            Assert.Equal("case.vfc", error.Diagnostic.Span.File);
            Assert.Equal(1, error.Diagnostic.Span.Line); return;
        }
        var tokens = Lexer.Tokenize(source, "case.vfc");
        Assert.Equal(TokenKind.End, tokens[^1].Kind);
        Assert.All(tokens, t => Assert.Equal("case.vfc", t.Span.File));
        foreach (var token in tokens.Where(t => t.Kind is not TokenKind.End and not TokenKind.NewLine))
            Assert.Equal(token.Text, source.Substring(token.Span.Start, token.Span.Length));
        if (iteration is 403 or 404 or 405) Assert.Equal(2, tokens[^1].Span.Line);
        if (iteration is 406 or 408) Assert.Equal(2, tokens.First(t => t.Text == "x").Span.Line);
    }
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void NewlineTokensRespectLimit(bool comment)
    {
        string source = new string('\n', 100001);
        if (comment) source = "/*" + source + "*/";
        Assert.Equal("L004", Assert.Throws<LanguageException>(() => Lexer.Tokenize(source)).Diagnostic.Code);
    }
}
