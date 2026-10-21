using System.Collections.ObjectModel;
using System.Numerics;

namespace VeloFormal.Language;
public sealed class TypedExpression
{
    public string Kind { get; }
    public LogicType Type { get; }
    public SourceSpan Span { get; }
    public string Text { get; }
    public BigInteger Value { get; }
    public IReadOnlyList<TypedExpression> Children { get; }
    internal TypedExpression(string kind, LogicType type, SourceSpan span, string text = "", BigInteger value = default,
        params TypedExpression[] children)
    {
        Kind = kind; Type = type; Span = span; Text = text; Value = value; Children = Array.AsReadOnly(children.ToArray());
    }
}
public sealed record BoundClause(ClauseRole Role, string Label, TypedExpression Expression, SourceSpan Span);
public sealed record BoundEnsure(string Label, string Operation, TypedExpression Trigger, TypedExpression Response,
    int Earliest, int Latest, SourceSpan Span);
public sealed class BoundContract
{
    public ContractSyntax Syntax { get; }
    public TransitionModel Model { get; }
    public TypedExpression Initial { get; }
    public TypedExpression Reset { get; }
    public IReadOnlyDictionary<string, TypedExpression> Next { get; }
    public IReadOnlyList<BoundClause> Clauses { get; }
    public IReadOnlyList<BoundEnsure> Ensures { get; }
    internal BoundContract(ContractSyntax syntax, TransitionModel model, TypedExpression initial, TypedExpression reset,
        Dictionary<string, TypedExpression> next, IEnumerable<BoundClause> clauses, IEnumerable<BoundEnsure> ensures)
    {
        Syntax = syntax; Model = model; Initial = initial; Reset = reset;
        Next = new ReadOnlyDictionary<string, TypedExpression>(new Dictionary<string, TypedExpression>(next, StringComparer.Ordinal));
        Clauses = Array.AsReadOnly(clauses.ToArray()); Ensures = Array.AsReadOnly(ensures.ToArray());
    }
}
public static class TypedEvaluation
{
    public static BigInteger Normalize(BigInteger value, LogicType type) => type.Kind == SortKind.Bits
        ? ((value % (BigInteger.One << type.Width)) + (BigInteger.One << type.Width)) % (BigInteger.One << type.Width) : value;
    private static BigInteger Signed(BigInteger value, LogicType type) => type.Kind == SortKind.Bits && type.Signed &&
        value >= (BigInteger.One << (type.Width - 1)) ? value - (BigInteger.One << type.Width) : value;
    public static BigInteger Evaluate(TypedExpression expression, Sample? sample = null, Sample? trigger = null)
    {
        BigInteger Child(int index) => Evaluate(expression.Children[index], sample, trigger);
        BigInteger Boolean(bool value) => value ? BigInteger.One : BigInteger.Zero;
        BigInteger value = expression.Kind switch
        {
            "constant" => expression.Value,
            "name" => (sample ?? throw new InputException("expression is not static"))[expression.Text],
            "capture" => Evaluate(expression.Children[0], trigger ?? throw new InputException("missing trigger sample"), null),
            "group" => Child(0),
            "cast" => Signed(Child(0), expression.Children[0].Type),
            "ite" => !Child(0).IsZero ? Child(1) : Child(2),
            "unary" => expression.Text switch { "!" => Boolean(Child(0).IsZero), "~" => ~Child(0), "-" => -Child(0), "+" => Child(0), _ => throw new InvalidOperationException() },
            "binary" => Binary(),
            _ => throw new InvalidOperationException()
        };
        return Normalize(value, expression.Type);

        BigInteger Binary()
        {
            var left = Child(0);
            if (expression.Text == "&&") return Boolean(!left.IsZero && !Child(1).IsZero);
            if (expression.Text == "||") return Boolean(!left.IsZero || !Child(1).IsZero);
            var right = Child(1);
            var signedLeft = Signed(left, expression.Children[0].Type);
            var signedRight = Signed(right, expression.Children[1].Type);
            return expression.Text switch
            {
                "+" => left + right, "-" => left - right, "*" => left * right, "&" => left & right, "|" => left | right, "^" => left ^ right,
                "==" => Boolean(left == right), "!=" => Boolean(left != right), "<" => Boolean(signedLeft < signedRight),
                "<=" => Boolean(signedLeft <= signedRight), ">" => Boolean(signedLeft > signedRight), ">=" => Boolean(signedLeft >= signedRight),
                "<<" => right >= expression.Type.Width ? BigInteger.Zero : left << (int)right,
                ">>" => right >= expression.Type.Width ? (expression.Type.Signed && signedLeft < 0 ? -1 : 0) : signedLeft >> (int)right,
                _ => throw new InvalidOperationException()
            };
        }
    }
}
