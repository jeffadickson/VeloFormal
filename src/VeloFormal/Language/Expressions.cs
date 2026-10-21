namespace VeloFormal.Language;

public static class LanguageExpressions
{
    public static ExprSyntax Parse(string source, string file = "<expression>")
    {
        var tokens = Lexer.Tokenize(source, file);
        var values = tokens.Where(t => t.Kind is not TokenKind.NewLine and not TokenKind.End).ToArray();
        if (values.Length == 0) throw new LanguageException(new("P006", "expected expression", tokens[^1].Span));
        return ParseTokens(values);
    }
    internal static ExprSyntax ParseTokens(IReadOnlyList<Token> tokens)
    {
        if (tokens.Count == 0) throw new LanguageException(new("P006", "expected expression", new("<expression>", 0, 0, 1, 1)));
        if (tokens.Count > 4096) throw new LanguageException(new("P009", "expression token limit exceeded", tokens[0].Span));
        var result = new Reader(tokens).Parse();
        var stack = new Stack<(ExprSyntax Node, int Depth)>();
        stack.Push((result, 1));
        while (stack.Count > 0)
        {
            var (node, depth) = stack.Pop();
            if (depth > 64) throw new LanguageException(new("P009", "AST depth exceeds 64", node.Span));
            IEnumerable<ExprSyntax> children = node switch
            {
                GroupedSyntax group => [group.Expression], UnarySyntax unary => [unary.Operand],
                BinarySyntax binary => [binary.Left, binary.Right], CallSyntax call => call.Arguments, _ => []
            };
            foreach (var child in children) stack.Push((child, depth + 1));
        }
        return result;
    }
    private sealed class Reader(IReadOnlyList<Token> tokens)
    {
        private int position, depth;
        private Token Current => position < tokens.Count ? tokens[position] :
            new(TokenKind.End, "", tokens[^1].Span with { Start = tokens[^1].Span.Start + tokens[^1].Span.Length, Length = 0, Column = tokens[^1].Span.Column + tokens[^1].Span.Length });
        private Token Next() { var token = Current; position++; return token; }
        private void Fail(string message) => throw new LanguageException(new("P006", message, Current.Span));
        private static SourceSpan Join(SourceSpan first, SourceSpan last) => first with { Length = last.Start + last.Length - first.Start };
        public ExprSyntax Parse()
        {
            var result = Binary(1);
            if (position != tokens.Count) Fail("unexpected expression token");
            return result;
        }
        private static int Priority(string op) => op switch
        {
            "||" => 1, "&&" => 2, "|" => 3, "^" => 4, "&" => 5, "==" or "!=" => 6,
            "<" or "<=" or ">" or ">=" => 7, "<<" or ">>" => 8, "+" or "-" => 9, "*" => 10, _ => 0
        };
        private ExprSyntax Binary(int minimum)
        {
            var left = Primary();
            while (Priority(Current.Text) >= minimum)
            {
                var op = Next();
                var right = Binary(Priority(op.Text) + 1);
                if (left is BinarySyntax previous && Priority(previous.Operator) is 6 or 7 && Priority(op.Text) is 6 or 7)
                    throw new LanguageException(new("P010", "chained comparisons require explicit boolean operators", op.Span));
                left = new BinarySyntax(op.Text, left, right, Join(left.Span, right.Span));
            }
            return left;
        }
        private ExprSyntax Primary()
        {
            if (++depth > 64) Fail("expression nesting exceeds 64");
            try
            {
                var token = Next();
                if (token.Text is "!" or "~" or "-" or "+")
                {
                    var value = Primary();
                    return new UnarySyntax(token.Text, value, Join(token.Span, value.Span));
                }
                if (token.Text == "(")
                {
                    var value = Binary(1);
                    if (Current.Text != ")") Fail("expected closing parenthesis");
                    var last = Next();
                    return new GroupedSyntax(value, Join(token.Span, last.Span));
                }
                if (token.Kind == TokenKind.Number || token.Text is "true" or "false")
                    return new LiteralSyntax(token.Text, token.Span);
                if (token.Kind != TokenKind.Identifier) Fail("expected expression");
                string name = token.Text;
                while (Current.Text == ".")
                {
                    Next();
                    if (Current.Kind != TokenKind.Identifier) Fail("expected path component");
                    name += "." + Next().Text;
                }
                if (Current.Text != "(") return new NameSyntax(name, Join(token.Span, tokens[position - 1].Span));
                Next();
                var arguments = new List<ExprSyntax>();
                if (Current.Text != ")")
                {
                    while (true)
                    {
                        arguments.Add(Binary(1));
                        if (Current.Text != ",") break;
                        Next();
                    }
                }
                if (Current.Text != ")") Fail("expected closing call parenthesis");
                var close = Next();
                return new CallSyntax(name, arguments.AsReadOnly(), Join(token.Span, close.Span));
            }
            finally { depth--; }
        }
    }
}
