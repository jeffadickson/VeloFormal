using System.Numerics;
using VeloFormal.Language;
using Xunit;
namespace VeloFormal.Tests;
public sealed class TypedEvaluationTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] text = ["count + 1", "count - 1", "count * 2", "~count", "-count", "signed(count) < 8'sd0",
            "unsigned(signed_count) > 8'd127", "resize(signed_count, 16)", "resize(count, 16)", "count << 1",
            "count >> 1", "signed_count >> 1", "8'b2", "8'hxz", "8'd256", "count + 256", "resize(count, 4)",
            "at_trigger(count)", "count == true", "8'o377"];
        int[] input = [255, 0, 128, 0, 1, 255, 255, 255, 255, 128, 128, 128, 0, 0, 0, 0, 0, 0, 0, 0];
        int[] expected = [0, 255, 0, 255, 255, 1, 1, 65535, 255, 0, 64, 192, -1, -1, -1, -1, -1, -1, -1, 255];
        return text.Select((source, index) => new object[] { 481 + index, source, input[index], expected[index] });
    }
    public static Sample Sample(int value) => new(new Dictionary<string, BigInteger>
        { ["count"] = value, ["signed_count"] = value, ["small"] = 0, ["enable"] = 1 });
    [Theory, MemberData(nameof(Cases))]
    public void FixedWidthSemantics(int iteration, string source, int input, int expected)
    {
        Assert.InRange(iteration, 481, 500);
        var binder = new SemanticBinder(SemanticTests.Model());
        var syntax = LanguageExpressions.Parse(source);
        if (expected < 0) { Assert.Throws<LanguageException>(() => binder.BindExpression(syntax)); return; }
        var expression = binder.BindExpression(syntax);
        Assert.Equal(new BigInteger(expected), TypedEvaluation.Evaluate(expression, Sample(input)));
    }
    [Theory]
    [InlineData("count << 8", 128, 0)]
    [InlineData("count >> 8", 128, 0)]
    [InlineData("signed_count >> 8", 128, 255)]
    [InlineData("ite(enable, count + 1, count)", 255, 0)]
    public void ShiftAndBranchEdges(string source, int input, int expected)
    {
        var expression = new SemanticBinder(SemanticTests.Model()).BindExpression(LanguageExpressions.Parse(source));
        Assert.Equal(new BigInteger(expected), TypedEvaluation.Evaluate(expression, Sample(input)));
    }
    [Fact]
    public void CapturedArithmeticWrapsAtTriggerWidth()
    {
        var expression = new SemanticBinder(SemanticTests.Model()).BindExpression(
            LanguageExpressions.Parse("count == at_trigger(count) + 1"), true);
        Assert.Equal(BigInteger.One, TypedEvaluation.Evaluate(expression, Sample(0), Sample(255)));
    }
}
