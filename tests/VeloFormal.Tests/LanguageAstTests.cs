using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class LanguageAstTests
{
    [Theory]
    [InlineData(421, "name")]
    [InlineData(422, "literal")]
    [InlineData(423, "unary")]
    [InlineData(424, "binary")]
    [InlineData(425, "call")]
    public void PreserveSourceIdentity(int iteration, string kind)
    {
        Assert.InRange(iteration, 421, 425);
        var span = new SourceSpan("source.vfc", 10, 5, 2, 3);
        ExprSyntax name = new NameSyntax("count", span);
        ExprSyntax node = kind switch
        {
            "literal" => new LiteralSyntax("8'd1", span),
            "unary" => new UnarySyntax("!", name, span),
            "binary" => new BinarySyntax("+", name, new LiteralSyntax("1", span), span),
            "call" => new CallSyntax("at_trigger", Array.AsReadOnly(new[] { name }), span),
            _ => name
        };
        Assert.Equal(span, node.Span);
        Assert.Equal("source.vfc", node.Span.File);
    }
}
