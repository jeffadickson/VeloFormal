using System.Globalization;
using System.Numerics;

namespace VeloFormal;

/// <summary>Restricted integer expression parser. No code evaluation or reflection.</summary>
public sealed record ExpressionInfo(IReadOnlyList<string> CurrentSignals, IReadOnlyList<string> TriggerSignals);

public static class Expression
{
    public static Func<Sample, Sample?, bool> Compile(string source, bool captures = false)
    {
        if (string.IsNullOrWhiteSpace(source) || source.Length > 4096)
            throw new InputException("expression must contain 1..4096 characters");
        var root = new Parser(source, captures).Parse();
        return (sample, capture) => root.Evaluate(sample, capture) != BigInteger.Zero;
    }

    public static ExpressionInfo Describe(string source, bool captures = false)
    {
        if (string.IsNullOrWhiteSpace(source) || source.Length > 4096)
            throw new InputException("expression must contain 1..4096 characters");
        var parser = new Parser(source, captures);
        parser.Parse();
        return new(Array.AsReadOnly(parser.CurrentSignals.Order(StringComparer.Ordinal).ToArray()),
            Array.AsReadOnly(parser.TriggerSignals.Order(StringComparer.Ordinal).ToArray()));
    }

    public static Func<Sample, bool> Predicate(string source)
    {
        var expression = Compile(source);
        return sample => expression(sample, null);
    }

    private static BigInteger Bool(bool value) => value ? BigInteger.One : BigInteger.Zero;
    private abstract record Node
    {
        public abstract BigInteger Evaluate(Sample sample, Sample? capture);
    }
    private sealed record Constant(BigInteger Value) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture) => Value;
    }
    private sealed record Signal(string Name) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture) => sample[Name];
    }
    private sealed record Capture(Node Value) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture) =>
            Value.Evaluate(capture ?? throw new InputException("at_trigger requires a trigger sample"), null);
    }
    private sealed record Unary(string Op, Node Value) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture)
        {
            var value = Value.Evaluate(sample, capture);
            return Op switch { "not" => Bool(value.IsZero), "-" => -value, "~" => ~value,
                _ => throw new InvalidOperationException() };
        }
    }
    private sealed record Binary(string Op, Node Left, Node Right) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture)
        {
            var left = Left.Evaluate(sample, capture);
            if (Op == "and") return Bool(!left.IsZero && !Right.Evaluate(sample, capture).IsZero);
            if (Op == "or") return Bool(!left.IsZero || !Right.Evaluate(sample, capture).IsZero);
            var right = Right.Evaluate(sample, capture);
            return Op switch { "+" => left + right, "-" => left - right, "&" => left & right,
                "|" => left | right, "^" => left ^ right, _ => throw new InvalidOperationException() };
        }
    }
    private sealed record Comparison(Node First, IReadOnlyList<(string Op, Node Right)> Rest) : Node
    {
        public override BigInteger Evaluate(Sample sample, Sample? capture)
        {
            var left = First.Evaluate(sample, capture);
            foreach (var (op, node) in Rest)
            {
                var right = node.Evaluate(sample, capture);
                var pass = op switch { "==" => left == right, "!=" => left != right,
                    "<" => left < right, "<=" => left <= right, ">" => left > right,
                    ">=" => left >= right, _ => throw new InvalidOperationException() };
                if (!pass) return BigInteger.Zero;
                left = right;
            }
            return BigInteger.One;
        }
    }

    private sealed class Parser(string source, bool captures)
    {
        public HashSet<string> CurrentSignals { get; } = new(StringComparer.Ordinal);
        public HashSet<string> TriggerSignals { get; } = new(StringComparer.Ordinal);
        private int position;
        private string token = "";
        private int depth;

        public Node Parse()
        {
            Next();
            var node = Or(false);
            if (token != "") Fail("unexpected token");
            return node;
        }
        private void Fail(string message) => throw new InputException($"{message} near character {position}: '{token}'");
        private void Next()
        {
            while (position < source.Length && char.IsWhiteSpace(source[position])) position++;
            if (position == source.Length) { token = ""; return; }
            int start = position++;
            char c = source[start];
            if (char.IsAsciiLetter(c))
            {
                while (position < source.Length && (char.IsAsciiLetterOrDigit(source[position]) || source[position] == '_')) position++;
            }
            else if (char.IsAsciiDigit(c))
            {
                while (position < source.Length && char.IsAsciiDigit(source[position])) position++;
            }
            else if ("<>=!".Contains(c) && position < source.Length && source[position] == '=') position++;
            else if (!"()+-&|^~<>".Contains(c))
                throw new InputException($"unsupported character '{c}' at {start}");
            token = source[start..position];
        }
        private bool Take(string value)
        {
            if (token != value) return false;
            Next();
            return true;
        }
        private Node Or(bool captured)
        {
            var node = And(captured);
            while (Take("or")) node = new Binary("or", node, And(captured));
            return node;
        }
        private Node And(bool captured)
        {
            var node = Not(captured);
            while (Take("and")) node = new Binary("and", node, Not(captured));
            return node;
        }
        private Node Not(bool captured)
        {
            Enter();
            try { return Take("not") ? new Unary("not", Not(captured)) : Compare(captured); }
            finally { depth--; }
        }
        private Node Compare(bool captured)
        {
            var first = BitOr(captured);
            var rest = new List<(string, Node)>();
            while (token is "==" or "!=" or "<" or "<=" or ">" or ">=")
            {
                var op = token; Next(); rest.Add((op, BitOr(captured)));
            }
            return rest.Count == 0 ? first : new Comparison(first, rest);
        }
        private Node BitOr(bool captured)
        {
            var node = BitXor(captured);
            while (Take("|")) node = new Binary("|", node, BitXor(captured));
            return node;
        }
        private Node BitXor(bool captured)
        {
            var node = BitAnd(captured);
            while (Take("^")) node = new Binary("^", node, BitAnd(captured));
            return node;
        }
        private Node BitAnd(bool captured)
        {
            var node = Add(captured);
            while (Take("&")) node = new Binary("&", node, Add(captured));
            return node;
        }
        private Node Add(bool captured)
        {
            var node = UnaryValue(captured);
            while (token is "+" or "-")
            {
                var op = token; Next(); node = new Binary(op, node, UnaryValue(captured));
            }
            return node;
        }
        private void Enter()
        {
            if (++depth > 64) throw new InputException("expression nesting exceeds 64");
        }
        private Node UnaryValue(bool captured)
        {
            Enter();
            try
            {
                if (token is "-" or "~")
                {
                    var op = token; Next(); return new Unary(op, UnaryValue(captured));
                }
                if (Take("("))
                {
                    var node = Or(captured);
                    if (!Take(")")) Fail("expected closing parenthesis");
                    return node;
                }
                if (Take("True")) return new Constant(BigInteger.One);
                if (Take("False")) return new Constant(BigInteger.Zero);
                if (token.Length > 0 && char.IsAsciiDigit(token[0]))
                {
                    var value = BigInteger.Parse(token, CultureInfo.InvariantCulture);
                    Next(); return new Constant(value);
                }
                if (Take("at_trigger"))
                {
                    if (!captures || captured) Fail("at_trigger is only allowed in responses and cannot nest");
                    if (!Take("(")) Fail("expected at_trigger argument");
                    var node = Or(true);
                    if (!Take(")")) Fail("expected closing parenthesis");
                    return new Capture(node);
                }
                if (token.Length > 0 && char.IsAsciiLetter(token[0]) && token is not "and" and not "or" and not "not")
                {
                    var name = token; Next();
                    (captured ? TriggerSignals : CurrentSignals).Add(name);
                    return new Signal(name);
                }
                Fail("expected expression");
                throw new InvalidOperationException();
            }
            finally { depth--; }
        }
    }
}
