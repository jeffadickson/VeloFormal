namespace VeloFormal.Language;

public readonly record struct SourceSpan(string File, int Start, int Length, int Line, int Column);
public sealed record Diagnostic(string Code, string Message, SourceSpan Span);
public sealed class LanguageException(Diagnostic diagnostic) : Exception(
    $"{diagnostic.Span.File}:{diagnostic.Span.Line}:{diagnostic.Span.Column}: {diagnostic.Code}: {diagnostic.Message}")
{
    public Diagnostic Diagnostic { get; } = diagnostic;
}
public enum TokenKind { Identifier, Number, Symbol, NewLine, End }
public sealed record Token(TokenKind Kind, string Text, SourceSpan Span);
