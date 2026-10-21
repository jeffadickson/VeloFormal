using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class SemanticTests
{
    public const string ModelJson = """
        {"module":"m","clock":"clk","signals":[
          {"name":"count","type":"unsigned","width":8,"kind":"state"},
          {"name":"small","type":"unsigned","width":4,"kind":"input"},
          {"name":"signed_count","type":"signed","width":8,"kind":"input"},
          {"name":"enable","type":"bool","kind":"input"}],
         "init":"count == 0","next":{"count":"count + 1"}}
        """;
    public static TransitionModel Model() => TransitionModel.Parse(ModelJson);
    public static IEnumerable<object[]> Cases()
    {
        string[] expressions = ["true", "count + 1 == count", "enable == (count == 0)", "unsigned(signed_count) == count",
            "signed(count) == signed_count", "count + 8'd255 == count", "ite(enable, count + 1, count) == count",
            "count == at_trigger(count) + 1", "missing == 0", "count == small", "count == signed_count",
            "enable + 1 == 2", "prev(count) == count", "at_trigger(at_trigger(count)) == count", "unknown(count) == count"];
        return expressions.Select((expression, index) => new object[] { 461 + index, expression, index < 8 });
    }
    [Theory, MemberData(nameof(Cases))]
    public void BindWidthsAndKinds(int iteration, string text, bool valid)
    {
        Assert.InRange(iteration, 461, 475);
        var binder = new SemanticBinder(Model());
        var expression = LanguageExpressions.Parse(text, "binding.vfc");
        if (!valid)
        {
            var error = Assert.Throws<LanguageException>(() => binder.BindExpression(expression, true, LogicType.Bool));
            Assert.Equal("binding.vfc", error.Diagnostic.Span.File); return;
        }
        Assert.Equal(LogicType.Bool, binder.BindExpression(expression, true, LogicType.Bool).Type);
    }
}
