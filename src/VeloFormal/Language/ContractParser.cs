namespace VeloFormal.Language;

/// <summary>Indentation-aware parser for the clocked v0.2 contract surface syntax.</summary>
public static class ContractParser
{
    public static ContractSyntax Parse(string source, string file = "<contract>") => new Reader(source, file).Parse();

    private sealed class Reader(string source, string file)
    {
        private readonly IReadOnlyList<Token> tokens = Lexer.Tokenize(source, file);
        private int position;
        private Token Current => tokens[position];
        private Token Next() { var value = Current; if (value.Kind != TokenKind.End) position++; return value; }
        private void Fail(string message, string code = "P001") => throw new LanguageException(new(code, message, Current.Span));
        private bool Take(string text) { if (Current.Text != text) return false; Next(); return true; }
        private Token Expect(string text) { if (Current.Text != text) Fail($"expected '{text}'"); return Next(); }
        private void Lines() { while (Current.Kind == TokenKind.NewLine) Next(); }
        private void EndLine()
        {
            if (Current.Kind is not TokenKind.NewLine and not TokenKind.End) Fail("unexpected trailing tokens");
            Lines();
        }
        private static readonly HashSet<string> Reserved = new(StringComparer.Ordinal)
        { "contract", "for", "clock", "posedge", "negedge", "reset", "when", "parameter", "integer", "where",
          "require", "invariant", "cover", "operation", "ensure", "after", "within", "cycles", "end", "true", "false" };
        private Token Name()
        {
            if (Current.Kind != TokenKind.Identifier || Reserved.Contains(Current.Text)) Fail("expected identifier");
            return Next();
        }
        private string Path()
        {
            string name = Name().Text;
            while (Take(".")) name += "." + Name().Text;
            return name;
        }
        private ExprSyntax Until(string delimiter)
        {
            var values = new List<Token>();
            int depth = 0;
            while (Current.Kind != TokenKind.End)
            {
                if (Current.Kind == TokenKind.NewLine)
                {
                    if (depth == 0) Fail($"expected '{delimiter}' before end of line");
                    Next(); continue;
                }
                if (depth == 0 && Current.Text == delimiter) break;
                if (Current.Text == "(") depth++;
                if (Current.Text == ")") depth--;
                values.Add(Next());
            }
            if (Current.Kind == TokenKind.End) Fail($"expected '{delimiter}'");
            if (values.Count == 0) Fail("expected expression");
            return LanguageExpressions.ParseTokens(values);
        }
        private ExprSyntax Body(int indent)
        {
            if (Current.Kind == TokenKind.NewLine)
            {
                Lines();
                if (Current.Kind == TokenKind.End || Current.Span.Column <= indent) Fail("expected indented expression");
            }
            var values = new List<Token>();
            int depth = 0;
            while (Current.Kind != TokenKind.End)
            {
                if (Current.Kind == TokenKind.NewLine)
                {
                    Lines();
                    if (depth == 0 && (Current.Kind == TokenKind.End || Current.Span.Column <= indent)) break;
                    continue;
                }
                if (Current.Text == "(") depth++;
                if (Current.Text == ")") depth--;
                values.Add(Next());
            }
            if (values.Count == 0) Fail("expected expression");
            return LanguageExpressions.ParseTokens(values);
        }
        public ContractSyntax Parse()
        {
            Lines();
            if (Current.Span.Column != 1) Fail("contract header must start in column 1");
            var start = Expect("contract");
            string name = Name().Text;
            Expect("for"); string module = Path(); EndLine();
            var parameters = new List<ParameterSyntax>();
            var clauses = new List<ClauseSyntax>();
            var operations = new List<OperationSyntax>();
            var labels = new HashSet<string>(StringComparer.Ordinal);
            var parameterNames = new HashSet<string>(StringComparer.Ordinal);
            var operationNames = new HashSet<string>(StringComparer.Ordinal);
            string? clock = null; SourceSpan clockSpan = start.Span; ExprSyntax? reset = null;
            int indent = Current.Span.Column;
            void Label(string label) { if (!labels.Add(label)) Fail($"duplicate clause label '{label}'", "P004"); }
            while (Current.Kind != TokenKind.End && Current.Text != "end")
            {
                if (Current.Span.Column <= 1 || Current.Span.Column != indent) Fail("inconsistent contract member indentation", "P003");
                var member = Next();
                switch (member.Text)
                {
                    case "clock":
                        if (clock is not null) Fail("duplicate clock", "P004");
                        Expect("posedge"); clockSpan = Current.Span; clock = Path(); EndLine(); break;
                    case "reset":
                        if (reset is not null) Fail("duplicate reset", "P004");
                        Expect("when"); reset = Body(indent); break;
                    case "parameter":
                        var parameter = Name();
                        if (!parameterNames.Add(parameter.Text)) Fail("duplicate parameter", "P004");
                        Expect(":"); Expect("integer"); Expect("where");
                        parameters.Add(new(parameter.Text, Body(indent), member.Span)); break;
                    case "require":
                    case "invariant":
                    case "cover":
                        var label = Name(); Label(label.Text); Expect(":");
                        clauses.Add(new(member.Text == "require" ? ClauseRole.Require :
                            member.Text == "invariant" ? ClauseRole.Invariant : ClauseRole.Cover,
                            label.Text, Body(indent), label.Span)); break;
                    case "operation":
                        var operation = Name();
                        if (!operationNames.Add(operation.Text)) Fail("duplicate operation", "P004");
                        Expect("when"); var trigger = Until(":"); Expect(":"); EndLine();
                        var ensures = new List<EnsureSyntax>();
                        int ensureIndent = Current.Span.Column;
                        while (Current.Kind != TokenKind.End && Current.Span.Column > indent)
                        {
                            if (Current.Span.Column != ensureIndent) Fail("inconsistent ensure indentation", "P003");
                            Expect("ensure"); var ensure = Name(); Label(ensure.Text);
                            ExprSyntax earliest, latest;
                            if (Take("after")) { earliest = latest = Until("cycles"); }
                            else if (Take("within")) { earliest = Until(".."); Expect(".."); latest = Until("cycles"); }
                            else { Fail("expected finite 'after' or 'within' delay", "P008"); throw new InvalidOperationException(); }
                            Expect("cycles"); Expect(":");
                            ensures.Add(new(ensure.Text, earliest, latest, Body(ensureIndent), ensure.Span));
                        }
                        if (ensures.Count == 0) Fail("operation needs at least one ensure", "P005");
                        operations.Add(new(operation.Text, trigger, ensures.AsReadOnly(), operation.Span)); break;
                    default: Fail($"unsupported contract member '{member.Text}'", "P008"); break;
                }
            }
            if (Current.Span.Column != 1) Fail("'end' must align with contract header", "P003");
            var end = Expect("end"); EndLine();
            if (Current.Kind != TokenKind.End) Fail("trailing content after contract", "P002");
            if (clock is null) throw new LanguageException(new("P005", "contract requires one posedge clock", start.Span));
            return new(name, module, clock, clockSpan, reset, parameters.AsReadOnly(), clauses.AsReadOnly(),
                operations.AsReadOnly(), start.Span with { Length = end.Span.Start + end.Span.Length - start.Span.Start }, source);
        }
    }
}
