using System.Globalization;
using System.Numerics;
using System.Text.RegularExpressions;

namespace VeloFormal.Language;
public sealed class SemanticBinder
{
    private readonly TransitionModel model;
    private readonly Dictionary<string, SignalDeclaration> signals;
    private int depth;
    public SemanticBinder(TransitionModel model)
    {
        this.model = model;
        signals = model.Signals.ToDictionary(s => s.Name, StringComparer.Ordinal);
    }
    private static LanguageException Error(ExprSyntax syntax, string message, string code = "B003") => new(new(code, message, syntax.Span));
    private static void Boolean(TypedExpression expression)
    {
        if (expression.Type != LogicType.Bool) throw new LanguageException(new("B003", "expected boolean expression", expression.Span));
    }
    public TypedExpression BindExpression(ExprSyntax syntax, bool allowCapture = false, LogicType? expected = null)
    {
        if (++depth > 128) throw Error(syntax, "binding nesting exceeds 128", "B009");
        try
        {
            var result = Coerce(Core(syntax, allowCapture), expected, syntax);
            if (result.Type.Kind == SortKind.Integer && result.Kind != "constant")
                throw Error(syntax, "mathematical integers are static-only; use explicitly sized values");
            return result;
        }
        finally { depth--; }
    }
    private TypedExpression Core(ExprSyntax syntax, bool capture) => syntax switch
    {
        LiteralSyntax literal => Literal(literal),
        NameSyntax name => model.Parameters.TryGetValue(name.Name, out var value)
            ? new("constant", LogicType.Integer, name.Span, value: value)
            : signals.TryGetValue(name.Name, out var signal) ? new("name", signal.Type, name.Span, name.Name)
            : throw Error(name, $"unknown signal or parameter '{name.Name}'", "B001"),
        GroupedSyntax group => Group(group, capture),
        UnarySyntax unary => Unary(unary, capture),
        BinarySyntax binary => Binary(binary, capture),
        CallSyntax call => Call(call, capture),
        _ => throw Error(syntax, "unsupported expression", "B008")
    };
    private static TypedExpression Literal(LiteralSyntax syntax)
    {
        if (syntax.Text is "true" or "false") return new("constant", LogicType.Bool, syntax.Span, value: syntax.Text == "true" ? 1 : 0);
        if (syntax.Text.Length > 4096) throw Error(syntax, "literal limit exceeded", "B009");
        if (!syntax.Text.Contains('\''))
        {
            if (!Regex.IsMatch(syntax.Text, @"^[0-9]+(?:_[0-9]+)*$")) throw Error(syntax, "invalid decimal literal", "B004");
            var value = BigInteger.Parse(syntax.Text.Replace("_", ""), CultureInfo.InvariantCulture);
            if (value.GetBitLength() > 4096) throw Error(syntax, "integer literal exceeds 4096 bits", "B009");
            return new("constant", LogicType.Integer, syntax.Span, value: value);
        }
        var match = Regex.Match(syntax.Text, @"^([0-9]+(?:_[0-9]+)*)'([sS]?)([bBoOdDhH])([0-9a-fA-F]+(?:_[0-9a-fA-F]+)*)$");
        if (!match.Success || !int.TryParse(match.Groups[1].Value.Replace("_", ""), out int width) || width is < 1 or > 256)
            throw Error(syntax, "invalid two-state sized literal or width", "B004");
        int radix = char.ToLowerInvariant(match.Groups[3].Value[0]) switch { 'b' => 2, 'o' => 8, 'd' => 10, _ => 16 };
        BigInteger number = 0;
        foreach (char digit in match.Groups[4].Value.Replace("_", "").ToLowerInvariant())
        {
            int value = digit is >= '0' and <= '9' ? digit - '0' : digit - 'a' + 10;
            if (value >= radix) throw Error(syntax, "digit outside literal base", "B004");
            number = number * radix + value;
        }
        if (number >= (BigInteger.One << width)) throw Error(syntax, "sized literal does not fit declared width", "B004");
        return new("constant", LogicType.Bits(width, match.Groups[2].Length != 0), syntax.Span, value: number);
    }
    private static TypedExpression Coerce(TypedExpression value, LogicType? expected, ExprSyntax syntax)
    {
        if (expected is null || value.Type == expected.Value) return value;
        var target = expected.Value;
        if (value.Type.Kind != SortKind.Integer || value.Kind != "constant" || target.Kind != SortKind.Bits)
            throw Error(syntax, "width or signedness mismatch; use explicit casts or resize");
        BigInteger min = target.Signed ? -(BigInteger.One << (target.Width - 1)) : 0;
        BigInteger max = (BigInteger.One << (target.Signed ? target.Width - 1 : target.Width)) - 1;
        if (value.Value < min || value.Value > max) throw Error(syntax, "integer constant does not fit operand type", "B004");
        return new("constant", target, value.Span, value: TypedEvaluation.Normalize(value.Value, target));
    }
    private static TypedExpression Make(string kind, LogicType type, ExprSyntax syntax, string text, params TypedExpression[] children)
    {
        var result = new TypedExpression(kind, type, syntax.Span, text, children: children);
        if (kind == "capture" && children[0].Kind == "constant") return new("constant", type, syntax.Span, value: children[0].Value);
        if (kind != "capture" && children.All(c => c.Kind == "constant"))
        {
            var value = TypedEvaluation.Evaluate(result);
            if (type.Kind == SortKind.Integer && value.GetBitLength() > 4096) throw Error(syntax, "static arithmetic exceeds 4096 bits", "B009");
            return new("constant", type, syntax.Span, value: value);
        }
        return result;
    }
    private TypedExpression Group(GroupedSyntax syntax, bool capture)
    {
        var value = BindExpression(syntax.Expression, capture);
        return Make("group", value.Type, syntax, "", value);
    }
    private TypedExpression Unary(UnarySyntax syntax, bool capture)
    {
        if (syntax.Operator is not "!" and not "~" and not "-" and not "+") throw Error(syntax, "unsupported unary operator", "B008");
        var value = BindExpression(syntax.Operand, capture);
        if (syntax.Operator == "!") Boolean(value);
        else if (value.Type.Kind == SortKind.Boolean) throw Error(syntax, "arithmetic unary operator needs a numeric operand");
        return Make("unary", syntax.Operator == "!" ? LogicType.Bool : value.Type, syntax, syntax.Operator, value);
    }
    private TypedExpression Binary(BinarySyntax syntax, bool capture)
    {
        if (syntax.Operator is not "+" and not "-" and not "*" and not "&" and not "|" and not "^" and not "==" and not "!=" and not "<" and not "<=" and not ">" and not ">=" and not "&&" and not "||" and not "<<" and not ">>") throw Error(syntax, "unsupported binary operator", "B008");
        var left = BindExpression(syntax.Left, capture);
        var right = BindExpression(syntax.Right, capture);
        bool logical = syntax.Operator is "&&" or "||";
        bool equality = syntax.Operator is "==" or "!=";
        bool comparison = equality || syntax.Operator is "<" or "<=" or ">" or ">=";
        if (logical) { Boolean(left); Boolean(right); }
        else
        {
            if (left.Type.Kind == SortKind.Integer && right.Type.Kind == SortKind.Bits) left = Coerce(left, right.Type, syntax.Left);
            if (right.Type.Kind == SortKind.Integer && left.Type.Kind == SortKind.Bits) right = Coerce(right, left.Type, syntax.Right);
            if (left.Type != right.Type) throw Error(syntax, "operands differ in width, signedness, or kind");
            if (left.Type.Kind == SortKind.Boolean && !equality) throw Error(syntax, "numeric operator cannot use booleans");
            if (syntax.Operator is "<<" or ">>" && left.Type.Kind != SortKind.Bits) throw Error(syntax, "shifts require fixed-width bitvectors");
        }
        return Make("binary", logical || comparison ? LogicType.Bool : left.Type, syntax, syntax.Operator, left, right);
    }
    private TypedExpression Call(CallSyntax syntax, bool capture)
    {
        if (syntax.Name == "at_trigger")
        {
            if (!capture || syntax.Arguments.Count != 1) throw Error(syntax, "at_trigger is response-only, unary, and cannot nest", "B008");
            var child = BindExpression(syntax.Arguments[0], false);
            return Make("capture", child.Type, syntax, "", child);
        }
        if (syntax.Name == "ite")
        {
            if (syntax.Arguments.Count != 3) throw Error(syntax, "ite needs condition and two values");
            var condition = BindExpression(syntax.Arguments[0], capture); Boolean(condition);
            var yes = BindExpression(syntax.Arguments[1], capture); var no = BindExpression(syntax.Arguments[2], capture);
            if (yes.Type.Kind == SortKind.Integer && no.Type.Kind == SortKind.Bits) yes = Coerce(yes, no.Type, syntax.Arguments[1]);
            if (no.Type.Kind == SortKind.Integer && yes.Type.Kind == SortKind.Bits) no = Coerce(no, yes.Type, syntax.Arguments[2]);
            if (yes.Type != no.Type) throw Error(syntax, "ite branches need matching types");
            return Make("ite", yes.Type, syntax, "", condition, yes, no);
        }
        if (syntax.Name is "unsigned" or "signed" or "resize")
        {
            int arity = syntax.Name == "resize" ? 2 : 1;
            if (syntax.Arguments.Count != arity) throw Error(syntax, "invalid cast argument count");
            var value = BindExpression(syntax.Arguments[0], capture);
            if (value.Type.Kind != SortKind.Bits) throw Error(syntax, "cast needs fixed-width bits");
            var type = value.Type with { Signed = syntax.Name == "resize" ? value.Type.Signed : syntax.Name == "signed" };
            if (arity == 2)
            {
                var width = BindExpression(syntax.Arguments[1]);
                if (width.Kind != "constant" || width.Type != LogicType.Integer || width.Value < value.Type.Width || width.Value > 256)
                    throw Error(syntax, "resize requires a static widening width up to 256", "B009");
                type = type with { Width = (int)width.Value };
            }
            return Make("cast", type, syntax, "", value);
        }
        throw Error(syntax, $"unsupported function '{syntax.Name}'", "B008");
    }
    private int Delay(ExprSyntax syntax)
    {
        var value = BindExpression(syntax);
        if (value.Kind != "constant" || value.Type != LogicType.Integer || value.Value < 1 || value.Value > int.MaxValue)
            throw Error(syntax, "delay must be a positive static integer", "B006");
        return (int)value.Value;
    }
    private static bool HasName(TypedExpression value) => value.Kind == "name" || value.Children.Any(HasName);
    public static BoundContract Bind(ContractSyntax syntax, TransitionModel model)
    {
        if (syntax.Module != model.Module || syntax.Clock != model.Clock)
            throw new LanguageException(new("B002", "module or clock binding mismatch", syntax.ClockSpan));
        var binder = new SemanticBinder(model);
        foreach (var parameter in syntax.Parameters)
        {
            if (!model.Parameters.ContainsKey(parameter.Name)) throw new LanguageException(new("B006", $"missing parameter '{parameter.Name}'", parameter.Span));
            var condition = binder.BindExpression(parameter.Constraint, expected: LogicType.Bool);
            if (HasName(condition) || TypedEvaluation.Evaluate(condition).IsZero)
                throw new LanguageException(new("B006", "parameter constraint is not static and satisfied", parameter.Span));
        }
        var initial = binder.BindExpression(model.Initial, expected: LogicType.Bool);
        var next = model.Next.ToDictionary(p => p.Key, p => binder.BindExpression(p.Value,
            expected: model.Signals.Single(s => s.Name == p.Key).Type), StringComparer.Ordinal);
        var reset = syntax.Reset is null ? new TypedExpression("constant", LogicType.Bool, syntax.ClockSpan)
            : binder.BindExpression(syntax.Reset, expected: LogicType.Bool);
        var clauses = syntax.Clauses.Select(c => new BoundClause(c.Role, c.Label,
            binder.BindExpression(c.Expression, expected: LogicType.Bool), c.Span)).ToArray();
        var ensures = new List<BoundEnsure>();
        foreach (var operation in syntax.Operations)
        {
            var trigger = binder.BindExpression(operation.Trigger, expected: LogicType.Bool);
            foreach (var ensure in operation.Ensures)
            {
                int earliest = binder.Delay(ensure.Earliest), latest = binder.Delay(ensure.Latest);
                if (earliest > latest) throw Error(ensure.Expression, "reversed delay interval", "B006");
                ensures.Add(new(ensure.Label, operation.Name, trigger, binder.BindExpression(ensure.Expression, true, LogicType.Bool),
                    earliest, latest, ensure.Span));
            }
        }
        return new(syntax, model, initial, reset, next, clauses, ensures);
    }
}
