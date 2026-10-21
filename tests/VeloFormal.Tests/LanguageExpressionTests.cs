using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class LanguageExpressionTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] source = ["1 + 2 * 3", "a || b && c", "!a && b", "(a + b) * c", "at_trigger(count) + 1",
            "unsigned(count) <= DEPTH", "8'hff", "a < b < c", "(a < b) == true", "1 +"];
        string[] root = ["+", "||", "&&", "*", "+", "<=", "literal", "error", "==", "error"];
        return source.Select((text, index) => new object[] { 441 + index, text, root[index] });
    }
    [Theory, MemberData(nameof(Cases))]
    public void PrecedenceAndGrouping(int iteration, string text, string expected)
    {
        Assert.InRange(iteration, 441, 450);
        if (expected == "error")
        {
            var diagnostic = Assert.Throws<LanguageException>(() => LanguageExpressions.Parse(text, "expr.vfc")).Diagnostic;
            Assert.Equal("expr.vfc", diagnostic.Span.File);
            if (text == "1 +") Assert.Equal(4, diagnostic.Span.Column);
            return;
        }
        var node = LanguageExpressions.Parse(text, "expr.vfc");
        Assert.Equal(expected, node is BinarySyntax binary ? binary.Operator : "literal");
        Assert.Equal(text, text.Substring(node.Span.Start, node.Span.Length));
        if (text == "1 + 2 * 3") Assert.Equal("*", Assert.IsType<BinarySyntax>(Assert.IsType<BinarySyntax>(node).Right).Operator);
    }
    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("a + a + a + a + a + a + a")]
    public void ResourceAndEmptyDiagnostics(string text)
    {
        if (text.StartsWith('a')) text = string.Join(" + ", Enumerable.Repeat("a", 100));
        Assert.Equal("limits.vfc", Assert.Throws<LanguageException>(() => LanguageExpressions.Parse(text, "limits.vfc")).Diagnostic.Span.File);
    }
}
