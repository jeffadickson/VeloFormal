using System.Numerics;
using Xunit;

namespace VeloFormal.Tests;

public sealed class RobustnessTests
{
    private static Sample S(int count = 0) => new(new Dictionary<string, BigInteger> { ["count"] = count });
    public static IEnumerable<object[]> Cases()
    {
        string[] names = ["long_expression", "deep_expression", "duplicate_labels", "blank_label",
            "zero_delay", "reverse_window", "reset_only", "empty", "untriggered", "invariant",
            "multiple_findings", "require_precedence", "snapshot", "label_case", "compound_capture",
            "capture_disallowed", "capture_missing", "xz_input", "exponent_input", "big_integer"];
        return names.Select((name, index) => new object[] { index + 171, name });
    }

    [Theory]
    [MemberData(nameof(Cases))]
    public void PreserveModelBoundaries(int iteration, string scenario)
    {
        Assert.InRange(iteration, 171, 190);
        switch (scenario)
        {
            case "long_expression": Assert.Throws<InputException>(() => Expression.Compile(new string('1', 4097))); break;
            case "deep_expression": Assert.Throws<InputException>(() => Expression.Compile(new string('(', 70) + "1" + new string(')', 70))); break;
            case "duplicate_labels": Assert.Throws<InputException>(() => new Contract(
                invariants: [new("a", _ => true)], covers: [new("a", _ => true)])); break;
            case "blank_label": Assert.Throws<InputException>(() => new Contract(invariants: [new(" ", _ => true)])); break;
            case "zero_delay": Assert.Throws<InputException>(() => new Contract(ensures: [new("a", _ => true, (_, _) => true, 0, 1)])); break;
            case "reverse_window": Assert.Throws<InputException>(() => new Contract(ensures: [new("a", _ => true, (_, _) => true, 2, 1)])); break;
            case "reset_only": Assert.Equal("incomplete", TraceChecker.Check(new(reset: _ => true), [S()]).Status); break;
            case "empty": Assert.Equal("incomplete", TraceChecker.Check(new(), []).Status); break;
            case "untriggered": Assert.Equal(["a"], TraceChecker.Check(new(ensures: [new("a", _ => false, (_, _) => true)]), [S()]).Untriggered); break;
            case "invariant": Assert.Equal("invariant", Assert.Single(TraceChecker.Check(new(invariants: [new("a", _ => false)]), [S()]).Findings).Kind); break;
            case "multiple_findings": Assert.Equal(2, TraceChecker.Check(new(invariants: [new("a", _ => false), new("b", _ => false)]), [S()]).Findings.Count); break;
            case "require_precedence":
                var invalid = TraceChecker.Check(new(requires: [new("r", _ => false)], invariants: [new("i", _ => false)]), [S()]);
                Assert.Equal("invalid-assumptions", invalid.Status); Assert.Equal("r", Assert.Single(invalid.Findings).Label); break;
            case "snapshot":
                var values = new Dictionary<string, BigInteger> { ["count"] = 1 };
                var sample = new Sample(values); values["count"] = 2;
                Assert.Equal(BigInteger.One, sample["count"]); break;
            case "label_case": Assert.Equal(2, new Contract(invariants: [new("A", _ => true), new("a", _ => true)]).Invariants.Count); break;
            case "compound_capture": Assert.True(Expression.Compile("count == at_trigger(count + 1)", true)(S(2), S(1))); break;
            case "capture_disallowed": Assert.Throws<InputException>(() => Expression.Predicate("at_trigger(count)")); break;
            case "capture_missing": Assert.Throws<InputException>(() => Expression.Compile("at_trigger(count)", true)(S(), null)); break;
            case "xz_input": Assert.Throws<InputException>(() => TraceDocument.Parse("""{"contract":{},"samples":[{"count":"Z"}]}""")); break;
            case "exponent_input": Assert.Throws<InputException>(() => TraceDocument.Parse("""{"contract":{},"samples":[{"count":1e2}]}""")); break;
            case "big_integer":
                var document = TraceDocument.Parse("""{"contract":{"invariants":{"large":"count == 99999999999999999999999999999999999999999"}},"samples":[{"count":99999999999999999999999999999999999999999}]}""");
                Assert.Equal("trace-pass", TraceChecker.Check(document.Contract, document.Samples).Status); break;
        }
    }
}
