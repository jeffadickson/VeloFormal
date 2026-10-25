using System.Text;

namespace VeloFormal.Language;

/// <summary>
/// Emits the canonical v0.2 spelling of a parsed contract. Formatting is deliberately
/// syntax-preserving: it never evaluates expressions or changes operation semantics.
/// </summary>
public static class ContractFormatter
{
    /// <summary>Formats a successfully parsed contract using four-space indentation and LF newlines.</summary>
    public static string Format(ContractSyntax contract)
    {
        ArgumentNullException.ThrowIfNull(contract);
        var text = new StringBuilder();
        text.Append("contract ").Append(contract.Name).Append(" for ").Append(contract.Module).AppendLine();
        foreach (var parameter in contract.Parameters)
            text.Append("    parameter ").Append(parameter.Name).Append(": integer where ").Append(Expression(parameter.Constraint)).AppendLine();
        text.Append("    clock posedge ").Append(contract.Clock).AppendLine();
        if (contract.Reset is not null) text.Append("    reset when ").Append(Expression(contract.Reset)).AppendLine();
        foreach (var clause in contract.Clauses)
            text.Append("    ").Append(Role(clause.Role)).Append(' ').Append(clause.Label).Append(": ").Append(Expression(clause.Expression)).AppendLine();
        foreach (var operation in contract.Operations)
        {
            text.Append("    operation ").Append(operation.Name).Append(" when ").Append(Expression(operation.Trigger)).AppendLine(":");
            foreach (var ensure in operation.Ensures)
            {
                text.Append("        ensure ").Append(ensure.Label).Append(' ');
                if (ExpressionsEqual(ensure.Earliest, ensure.Latest)) text.Append("after ").Append(Expression(ensure.Earliest));
                else text.Append("within ").Append(Expression(ensure.Earliest)).Append("..").Append(Expression(ensure.Latest));
                text.AppendLine(" cycles:");
                text.Append("            ").Append(Expression(ensure.Expression)).AppendLine();
            }
        }
        text.AppendLine("end");
        return text.ToString().ReplaceLineEndings("\n");
    }

    private static string Role(ClauseRole role) => role switch
    {
        ClauseRole.Require => "require", ClauseRole.Invariant => "invariant", ClauseRole.Cover => "cover",
        _ => throw new ArgumentOutOfRangeException(nameof(role))
    };

    private static bool ExpressionsEqual(ExprSyntax left, ExprSyntax right) => Expression(left) == Expression(right);

    private static string Expression(ExprSyntax expression) => Expression(expression, 0, false);

    private static string Expression(ExprSyntax expression, int parentPrecedence, bool rightChild) => expression switch
    {
        LiteralSyntax literal => literal.Text,
        NameSyntax name => name.Name,
        GroupedSyntax group => Expression(group.Expression, parentPrecedence, rightChild),
        UnarySyntax unary => Wrap(unary.Operator + Expression(unary.Operand, 11, false), 11, parentPrecedence, rightChild),
        BinarySyntax binary => Binary(binary, parentPrecedence, rightChild),
        CallSyntax call => call.Name + "(" + string.Join(", ", call.Arguments.Select(argument => Expression(argument))) + ")",
        _ => throw new ArgumentOutOfRangeException(nameof(expression))
    };
    private static string Binary(BinarySyntax binary, int parentPrecedence, bool rightChild)
    {
        int precedence = Precedence(binary.Operator);
        string left = Expression(binary.Left, precedence, false);
        string right = Expression(binary.Right, precedence, true);
        if (IsComparison(binary.Operator) && Unwrap(binary.Left) is BinarySyntax leftBinary && IsComparison(leftBinary.Operator)) left = "(" + Expression(binary.Left) + ")";
        if (IsComparison(binary.Operator) && Unwrap(binary.Right) is BinarySyntax rightBinary && IsComparison(rightBinary.Operator)) right = "(" + Expression(binary.Right) + ")";
        string value = left + " " + binary.Operator + " " + right;
        return Wrap(value, precedence, parentPrecedence, rightChild);
    }
    private static string Wrap(string value, int precedence, int parentPrecedence, bool rightChild) =>
        precedence < parentPrecedence || (rightChild && precedence == parentPrecedence) ? "(" + value + ")" : value;
    private static ExprSyntax Unwrap(ExprSyntax expression) => expression is GroupedSyntax group ? Unwrap(group.Expression) : expression;

    private static bool IsComparison(string op) => op is "==" or "!=" or "<" or "<=" or ">" or ">=";

    private static int Precedence(string op) => op switch
    {
        "||" => 1, "&&" => 2, "|" => 3, "^" => 4, "&" => 5, "==" or "!=" => 6,
        "<" or "<=" or ">" or ">=" => 7, "<<" or ">>" => 8, "+" or "-" => 9, "*" => 10,
        _ => throw new ArgumentOutOfRangeException(nameof(op))
    };
    private static string ParenthesizeUnary(ExprSyntax operand) => operand switch
    {
        LiteralSyntax or NameSyntax or CallSyntax or GroupedSyntax or UnarySyntax => Expression(operand),
        _ => "(" + Expression(operand) + ")"
    };
}
