namespace VeloFormal.Language;

public abstract record ExprSyntax(SourceSpan Span);
public sealed record LiteralSyntax(string Text, SourceSpan Span) : ExprSyntax(Span);
public sealed record NameSyntax(string Name, SourceSpan Span) : ExprSyntax(Span);
public sealed record UnarySyntax(string Operator, ExprSyntax Operand, SourceSpan Span) : ExprSyntax(Span);
public sealed record BinarySyntax(string Operator, ExprSyntax Left, ExprSyntax Right, SourceSpan Span) : ExprSyntax(Span);
public sealed record CallSyntax(string Name, IReadOnlyList<ExprSyntax> Arguments, SourceSpan Span) : ExprSyntax(Span);
public enum ClauseRole { Require, Invariant, Cover }
public sealed record ParameterSyntax(string Name, ExprSyntax Constraint, SourceSpan Span);
public sealed record ClauseSyntax(ClauseRole Role, string Label, ExprSyntax Expression, SourceSpan Span);
public sealed record EnsureSyntax(string Label, ExprSyntax Earliest, ExprSyntax Latest, ExprSyntax Expression, SourceSpan Span);
public sealed record OperationSyntax(string Name, ExprSyntax Trigger, IReadOnlyList<EnsureSyntax> Ensures, SourceSpan Span);
public sealed record ContractSyntax(string Name, string Module, string Clock, SourceSpan ClockSpan, ExprSyntax? Reset,
    IReadOnlyList<ParameterSyntax> Parameters, IReadOnlyList<ClauseSyntax> Clauses,
    IReadOnlyList<OperationSyntax> Operations, SourceSpan Span, string Source);
