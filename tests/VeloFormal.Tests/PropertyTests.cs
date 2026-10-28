using FsCheck;
using FsCheck.Xunit;
using VeloFormal.Language;
using Xunit;

namespace VeloFormal.Tests;

/// <summary>Property-based invariants for canonical parser output with reproducible FsCheck input.</summary>
public sealed class PropertyTests
{
    /// <summary>
    /// A generated arithmetic chain must preserve its canonical formatting under a second parse/format pass.
    /// The input is intentionally bounded to the documented AST depth and each failing case is reproducible
    /// through the fixed FsCheck replay seed.
    /// </summary>
    [Property(MaxTest = 100, Replay = "123456,789013")]
    public bool FormattingArithmeticChainsIsIdempotent(NonEmptyArray<byte> terms)
    {
        string expression = string.Join(" + ", terms.Get.Take(32).Select(value => (value % 16).ToString()));
        string source = $"contract Generated for m\n    clock posedge clk\n    invariant safe: count + {expression} >= count\nend";
        string once = ContractFormatter.Format(ContractParser.Parse(source, "generated.vfc"));
        string twice = ContractFormatter.Format(ContractParser.Parse(once, "formatted.vfc"));
        return once == twice;
    }

    /// <summary>
    /// Canonical formatting must retain the bound expression's result for representative
    /// two-state samples. This independently exercises precedence, grouping, casts, and
    /// conditional formatting rather than merely comparing two formatter invocations.
    /// </summary>
    [Property(MaxTest = 100, Replay = "246813,135791")]
    public bool FormattingPreservesBoundExpressionEvaluation(NonNegativeInt selection, byte count, byte small,
        byte signedCount, bool enable)
    {
        string[] expressions =
        [
            "count + 1 == count", "(count < 8'd128) == enable",
            "ite(enable, count + 1, count) == count", "count >> 1 <= count",
            "(count & 8'h0f) <= count", "count == count + 8'd0",
            "(count < 8'd10) || (count >= 8'd10)", "signed(count) >> 1 <= signed(count)"
        ];
        string expression = expressions[selection.Get % expressions.Length];
        string source = $"contract Generated for m\n    clock posedge clk\n    invariant stable: {expression}\nend";
        var original = ContractParser.Parse(source, "generated.vfc");
        var formatted = ContractParser.Parse(ContractFormatter.Format(original), "formatted.vfc");
        var binder = new SemanticBinder(SemanticTests.Model());
        var sample = new Sample(new Dictionary<string, System.Numerics.BigInteger>
        {
            ["count"] = count, ["small"] = small, ["signed_count"] = signedCount, ["enable"] = enable ? 1 : 0
        });
        var before = binder.BindExpression(Assert.Single(original.Clauses).Expression, expected: LogicType.Bool);
        var after = binder.BindExpression(Assert.Single(formatted.Clauses).Expression, expected: LogicType.Bool);
        return TypedEvaluation.Evaluate(before, sample) == TypedEvaluation.Evaluate(after, sample);
    }
}
