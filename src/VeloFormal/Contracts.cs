using System.Collections.ObjectModel;
using System.Numerics;

namespace VeloFormal;

public sealed class InputException(string message) : Exception(message);

/// <summary>Immutable two-state sample. Integers have mathematical, not RTL, width.</summary>
public sealed class Sample
{
    private readonly IReadOnlyDictionary<string, BigInteger> values;
    public Sample(IEnumerable<KeyValuePair<string, BigInteger>> values) =>
        this.values = new ReadOnlyDictionary<string, BigInteger>(values.ToDictionary(p => p.Key, p => p.Value));
    public BigInteger this[string name] => values.TryGetValue(name, out var value)
        ? value : throw new InputException($"missing signal: {name}");
}

public sealed record PredicateClause(string Label, Func<Sample, bool> Test);
public sealed record EnsureClause(string Label, Func<Sample, bool> Trigger,
    Func<Sample, Sample, bool> Response, int Earliest = 1, int Latest = 1);

public sealed class Contract
{
    public IReadOnlyList<PredicateClause> Requires { get; }
    public IReadOnlyList<PredicateClause> Invariants { get; }
    public IReadOnlyList<EnsureClause> Ensures { get; }
    public IReadOnlyList<PredicateClause> Covers { get; }
    public Func<Sample, bool> Reset { get; }

    public Contract(IEnumerable<PredicateClause>? requires = null,
        IEnumerable<PredicateClause>? invariants = null, IEnumerable<EnsureClause>? ensures = null,
        IEnumerable<PredicateClause>? covers = null, Func<Sample, bool>? reset = null)
    {
        Requires = Array.AsReadOnly((requires ?? []).ToArray());
        Invariants = Array.AsReadOnly((invariants ?? []).ToArray());
        Ensures = Array.AsReadOnly((ensures ?? []).ToArray());
        Covers = Array.AsReadOnly((covers ?? []).ToArray());
        Reset = reset ?? (_ => false);
        var labels = Requires.Concat(Invariants).Concat(Covers).Select(c => c.Label)
            .Concat(Ensures.Select(c => c.Label)).ToArray();
        if (labels.Any(string.IsNullOrWhiteSpace)) throw new InputException("clause labels must be nonempty");
        if (labels.Distinct(StringComparer.Ordinal).Count() != labels.Length)
            throw new InputException("clause labels must be unique");
        if (Ensures.Any(c => c.Earliest < 1 || c.Latest < c.Earliest))
            throw new InputException("delay bounds must satisfy 1 <= earliest <= latest");
    }
}

public sealed record Finding(string Label, int Sample, string Kind, int? TriggerSample = null);
public sealed record PendingObligation(string Label, int TriggerSample, long EarliestSample, long DeadlineSample);
public sealed record Report(string Status, IReadOnlyList<Finding> Findings, int Pending,
    int Cancelled, int ActiveSamples, IReadOnlyList<string> Untriggered, IReadOnlyList<string> Covered)
{
    public int ObservedSamples { get; init; }
    public int ResetSamples { get; init; }
    public IReadOnlyList<PendingObligation> Obligations { get; init; } = Array.Empty<PendingObligation>();
}

public static class TraceChecker
{
    private sealed record Obligation(EnsureClause Clause, int Start, Sample Capture);

    public static Report Check(Contract contract, IEnumerable<Sample> samples, CheckOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(contract);
        ArgumentNullException.ThrowIfNull(samples);
        options ??= new();
        options.Validate();
        cancellationToken.ThrowIfCancellationRequested();
        var pending = new List<Obligation>();
        var findings = new List<Finding>();
        var triggered = new HashSet<string>(StringComparer.Ordinal);
        var covered = new HashSet<string>(StringComparer.Ordinal);
        int cancelled = 0, active = 0, tick = -1;
        bool invalid = false;
        bool Evaluate(string label, Func<bool> predicate)
        {
            cancellationToken.ThrowIfCancellationRequested();
            try { bool result = predicate(); cancellationToken.ThrowIfCancellationRequested(); return result; }
            catch (InputException error) { throw new InputException($"clause {label}, sample {tick}: {error.Message}"); }
        }
        void AddFinding(Finding finding)
        {
            if (findings.Count >= options.MaxFindings) throw new InputException("finding limit exceeded");
            findings.Add(finding);
        }
        foreach (var sample in samples)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (tick + 1 >= options.MaxSamples) throw new InputException("sample limit exceeded");
            tick = checked(tick + 1);
            if (Evaluate("<reset>", () => contract.Reset(sample)))
            {
                cancelled = checked(cancelled + pending.Count);
                pending.Clear();
                continue;
            }
            active++;
            var failures = contract.Requires.Where(c => !Evaluate(c.Label, () => c.Test(sample))).ToArray();
            if (failures.Length != 0)
            {
                foreach (var failure in failures) AddFinding(new(failure.Label, tick, "require"));
                invalid = true;
                break;
            }
            foreach (var clause in contract.Invariants)
                if (!Evaluate(clause.Label, () => clause.Test(sample))) AddFinding(new(clause.Label, tick, "invariant"));
            var remaining = new List<Obligation>();
            foreach (var obligation in pending)
            {
                var clause = obligation.Clause;
                int age = tick - obligation.Start;
                if (age >= clause.Earliest && Evaluate(clause.Label,
                    () => clause.Response(sample, obligation.Capture))) continue;
                if (age == clause.Latest)
                    AddFinding(new(clause.Label, tick, "ensure", obligation.Start));
                else remaining.Add(obligation);
            }
            pending = remaining;
            foreach (var clause in contract.Ensures)
                if (Evaluate(clause.Label, () => clause.Trigger(sample)))
                {
                    triggered.Add(clause.Label);
                    if (pending.Count >= options.MaxPending) throw new InputException("pending obligation limit exceeded");
                    pending.Add(new(clause, tick, sample));
                }
            foreach (var clause in contract.Covers)
                if (Evaluate(clause.Label, () => clause.Test(sample))) covered.Add(clause.Label);
        }
        cancellationToken.ThrowIfCancellationRequested();
        var status = invalid ? "invalid-assumptions" : findings.Count > 0 ? "counterexample"
            : pending.Count > 0 || active == 0 ? "incomplete" : "trace-pass";
        return new(status, findings.AsReadOnly(), pending.Count, cancelled, active,
            Array.AsReadOnly(contract.Ensures.Where(c => !triggered.Contains(c.Label)).Select(c => c.Label).ToArray()),
            Array.AsReadOnly(contract.Covers.Where(c => covered.Contains(c.Label)).Select(c => c.Label).ToArray()))
        {
            ObservedSamples = tick + 1,
            ResetSamples = tick + 1 - active,
            Obligations = Array.AsReadOnly(pending.Select(o => new PendingObligation(o.Clause.Label,
                o.Start, (long)o.Start + o.Clause.Earliest, (long)o.Start + o.Clause.Latest)).ToArray())
        };
    }
}
