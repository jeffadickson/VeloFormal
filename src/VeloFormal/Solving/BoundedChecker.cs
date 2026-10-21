using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Globalization;
using Microsoft.Z3;
using VeloFormal.Language;

namespace VeloFormal.Solving;
public sealed record SolverOptions(int Depth = 16, uint TimeoutMilliseconds = 10000, int MaxExpressions = 200000, int MaxQueries = 4096)
{
    internal void Validate()
    {
        if (Depth is < 0 or > 128 || TimeoutMilliseconds is < 1 or > 60000 || MaxExpressions is < 1 or > 2000000 || MaxQueries is < 1 or > 10000)
            throw new InputException("invalid solver depth, timeout, or resource limit");
    }
}
public sealed record SolverFinding(string Label, SourceSpan Span, int Sample, int? TriggerSample = null);
public sealed record SolverFrame(int Sample, IReadOnlyDictionary<string, string> Values);
public sealed record GoalObservation(string Label, string Status);
public sealed record SolverReport(string Status, int Depth, string Engine, string Contract, string Module, string Clock,
    IReadOnlyList<SolverFinding> Findings, IReadOnlyList<SolverFrame> Counterexample,
    IReadOnlyList<GoalObservation> Covers, IReadOnlyList<GoalObservation> Triggers,
    IReadOnlyList<string> PendingLabels, string? Reason = null)
{
    public bool IsProof => false;
}
public static class BoundedChecker
{
    public static SolverReport Check(BoundContract contract, SolverOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(contract);
        options ??= new(); options.Validate(); cancellationToken.ThrowIfCancellationRequested();
        if (contract.Clauses.Count + contract.Ensures.Count > 1024) throw new InputException("property limit exceeded");
        if (contract.Ensures.Count != 0) throw new InputException("temporal backend not enabled in this intermediate checkpoint");
        using var context = new Context();
        using var registration = cancellationToken.Register(context.Interrupt);
        using var solver = context.MkSolver();
        var lowering = new Z3Translation(context, contract.Model, options.MaxExpressions);
        var stopwatch = Stopwatch.StartNew();
        int queries = 0;
        string engine = $"Z3 {Microsoft.Z3.Version.Major}.{Microsoft.Z3.Version.Minor}.{Microsoft.Z3.Version.Build}";
        SolverReport Report(string status, string? reason = null, IReadOnlyList<SolverFinding>? findings = null, IReadOnlyList<SolverFrame>? frames = null) =>
            new(status, options.Depth, engine, contract.Syntax.Name, contract.Model.Module, contract.Model.Clock,
                findings ?? Array.Empty<SolverFinding>(), frames ?? Array.Empty<SolverFrame>(),
                Array.Empty<GoalObservation>(), Array.Empty<GoalObservation>(), Array.Empty<string>(), reason);
        Status Query()
        {
            cancellationToken.ThrowIfCancellationRequested();
            long remaining = options.TimeoutMilliseconds - stopwatch.ElapsedMilliseconds;
            if (++queries > options.MaxQueries || remaining <= 0) throw new SolverBudgetException("query or total time budget exceeded");
            using var parameters = context.MkParams();
            parameters.Add("timeout", (uint)Math.Max(1, remaining));
            solver.Parameters = parameters;
            var result = solver.Check();
            cancellationToken.ThrowIfCancellationRequested();
            return result;
        }
        IReadOnlyList<SolverFrame> Frames(Model model, int last)
        {
            var frames = new List<SolverFrame>();
            for (int time = 0; time <= last; time++)
            {
                var values = new Dictionary<string, string>(StringComparer.Ordinal);
                foreach (var signal in contract.Model.Signals)
                {
                    var value = model.Evaluate(lowering.Variable(signal, time), true);
                    values.Add(signal.Name, signal.Type.Kind == SortKind.Boolean ? (value.IsTrue ? "1" : "0")
                        : ((BitVecNum)value).BigInteger.ToString(CultureInfo.InvariantCulture));
                }
                frames.Add(new(time, new ReadOnlyDictionary<string, string>(values)));
            }
            return frames.AsReadOnly();
        }
        try
        {
            solver.Assert(lowering.Boolean(contract.Initial, 0));
            var consistency = Query();
            if (consistency == Status.UNKNOWN) return Report("unknown", solver.ReasonUnknown);
            if (consistency == Status.UNSATISFIABLE) return Report("model-unsat", "initial state constraints are inconsistent");
            for (int time = 0; time <= options.Depth; time++)
            {
                cancellationToken.ThrowIfCancellationRequested();
                if (time > 0)
                    foreach (var signal in contract.Model.Signals.Where(s => s.IsState))
                        solver.Assert(context.MkEq(lowering.Variable(signal, time), lowering.Translate(contract.Next[signal.Name], time - 1)));
                var active = context.MkNot(lowering.Boolean(contract.Reset, time));
                foreach (var requirement in contract.Clauses.Where(c => c.Role == ClauseRole.Require))
                    solver.Assert(context.MkImplies(active, lowering.Boolean(requirement.Expression, time)));
                var feasible = Query();
                if (feasible == Status.UNKNOWN) return Report("unknown", solver.ReasonUnknown);
                if (feasible == Status.UNSATISFIABLE) return Report("assumption-unsat", $"no feasible prefix through sample {time}");
                var violations = contract.Clauses.Where(c => c.Role == ClauseRole.Invariant)
                    .Select(c => (Finding: new SolverFinding(c.Label, c.Span, time),
                        Formula: context.MkAnd(active, context.MkNot(lowering.Boolean(c.Expression, time))))).ToList();
                if (violations.Count == 0) continue;
                solver.Push();
                try
                {
                    solver.Assert(context.MkOr(violations.Select(v => v.Formula).ToArray()));
                    var status = Query();
                    if (status == Status.UNKNOWN) return Report("unknown", solver.ReasonUnknown);
                    if (status == Status.SATISFIABLE)
                    {
                        using var model = solver.Model;
                        var findings = violations.Where(v => model.Evaluate(v.Formula, true).IsTrue).Select(v => v.Finding).ToArray();
                        return Report("counterexample", findings: Array.AsReadOnly(findings), frames: Frames(model, time));
                    }
                }
                finally { solver.Pop(); }
            }
            return Report("bounded-pass");
        }
        catch (SolverBudgetException error) { return Report("unknown", error.Message); }
    }
}
