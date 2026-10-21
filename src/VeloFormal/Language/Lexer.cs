namespace VeloFormal.Language;

public static class Lexer
{
    public static IReadOnlyList<Token> Tokenize(string source, string file = "<contract>")
    {
        ArgumentNullException.ThrowIfNull(source);
        var tokens = new List<Token>();
        int offset = 0, line = 1, column = 1;
        SourceSpan Span(int start, int length, int row, int col) => new(file, start, length, row, col);
        void Error(string code, string message, int start, int row, int col) =>
            throw new LanguageException(new(code, message, Span(start, Math.Max(1, offset - start), row, col)));
        if (source.Length > 1048576) Error("L004", "source exceeds 1 MiB", 0, 1, 1);
        void Advance() { offset++; column++; }
        void NewLine()
        {
            int start = offset, row = line, col = column;
            if (source[offset] == '\r' && offset + 1 < source.Length && source[offset + 1] == '\n') offset++;
            offset++; line++; column = 1;
            if (tokens.Count >= 100000) Error("L004", "token limit exceeded", start, row, col);
            tokens.Add(new(TokenKind.NewLine, "\n", Span(start, offset - start, row, col)));
        }
        while (offset < source.Length)
        {
            char c = source[offset];
            if (c == ' ') { Advance(); continue; }
            if (c == '\t') Error("L005", "use spaces instead of tabs", offset, line, column);
            if (c is '\r' or '\n') { NewLine(); continue; }
            int start = offset, row = line, col = column;
            if (c == '/' && offset + 1 < source.Length && source[offset + 1] == '/')
            {
                while (offset < source.Length && source[offset] is not '\r' and not '\n') Advance();
                continue;
            }
            if (c == '/' && offset + 1 < source.Length && source[offset + 1] == '*')
            {
                Advance(); Advance();
                while (offset < source.Length && !(source[offset] == '*' && offset + 1 < source.Length && source[offset + 1] == '/'))
                {
                    if (source[offset] is '\r' or '\n') NewLine(); else Advance();
                }
                if (offset == source.Length) Error("L002", "unterminated block comment", start, row, col);
                Advance(); Advance(); continue;
            }
            TokenKind kind;
            if (char.IsAsciiLetter(c) || c == '_')
            {
                kind = TokenKind.Identifier;
                do Advance(); while (offset < source.Length && (char.IsAsciiLetterOrDigit(source[offset]) || source[offset] == '_'));
            }
            else if (char.IsAsciiDigit(c))
            {
                kind = TokenKind.Number;
                do Advance(); while (offset < source.Length && (char.IsAsciiDigit(source[offset]) || source[offset] == '_'));
                if (offset < source.Length && source[offset] == '\'')
                {
                    Advance();
                    if (offset < source.Length && source[offset] is 's' or 'S') Advance();
                    if (offset == source.Length || !"bBoOdDhH".Contains(source[offset]))
                        Error("L003", "expected literal base b, o, d, or h", start, row, col);
                    Advance();
                    int digits = offset;
                    while (offset < source.Length && (char.IsAsciiLetterOrDigit(source[offset]) || source[offset] == '_')) Advance();
                    if (offset == digits) Error("L003", "literal requires digits", start, row, col);
                }
            }
            else
            {
                kind = TokenKind.Symbol;
                string pair = offset + 1 < source.Length ? source.Substring(offset, 2) : "";
                if (pair is "&&" or "||" or "==" or "!=" or "<=" or ">=" or ".." or "<<" or ">>")
                { Advance(); Advance(); }
                else if ("()[]:,+-*/%&|^!~<>.=;".Contains(c)) Advance();
                else Error("L001", $"unsupported character '{c}'", start, row, col);
            }
            tokens.Add(new(kind, source[start..offset], Span(start, offset - start, row, col)));
            if (tokens.Count > 100000) Error("L004", "token limit exceeded", start, row, col);
        }
        tokens.Add(new(TokenKind.End, "", Span(offset, 0, line, column)));
        return tokens.AsReadOnly();
    }
}
