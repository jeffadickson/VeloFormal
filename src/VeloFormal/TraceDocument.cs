using System.Globalization;
using System.Numerics;
using System.Text.Json;

namespace VeloFormal;

public sealed record TraceDocument(Contract Contract, IReadOnlyList<Sample> Samples)
{
    public static TraceDocument Parse(string json)
    {
        try
        {
            using var document = JsonDocument.Parse(json, new JsonDocumentOptions { MaxDepth = 64 });
            var root = document.RootElement;
            Object(root, "document", ["contract", "samples"], ["contract", "samples"]);
            var spec = root.GetProperty("contract");
            Object(spec, "contract", ["requires", "invariants", "ensures", "covers", "reset"]);
            var ensures = new List<EnsureClause>();
            if (spec.TryGetProperty("ensures", out var array))
            {
                Array(array, "ensures");
                foreach (var clause in array.EnumerateArray())
                {
                    Object(clause, "ensure", ["label", "when", "response", "after", "within"], ["label", "when", "response"]);
                    bool after = clause.TryGetProperty("after", out var exact);
                    bool within = clause.TryGetProperty("within", out var window);
                    if (after == within) throw new InputException("ensure must specify exactly one of after or within");
                    int earliest, latest;
                    if (after) earliest = latest = Delay(exact);
                    else
                    {
                        Array(window, "within");
                        if (window.GetArrayLength() != 2) throw new InputException("within must have two bounds");
                        earliest = Delay(window[0]); latest = Delay(window[1]);
                    }
                    var response = Expression.Compile(Text(clause.GetProperty("response")), true);
                    ensures.Add(new(Text(clause.GetProperty("label")),
                        Expression.Predicate(Text(clause.GetProperty("when"))),
                        (sample, capture) => response(sample, capture), earliest, latest));
                }
            }
            var contract = new Contract(Predicates(spec, "requires"), Predicates(spec, "invariants"),
                ensures, Predicates(spec, "covers"),
                Expression.Predicate(spec.TryGetProperty("reset", out var reset) ? Text(reset) : "False"));
            var samples = root.GetProperty("samples");
            Array(samples, "samples");
            var result = new List<Sample>();
            foreach (var sample in samples.EnumerateArray())
            {
                if (sample.ValueKind != JsonValueKind.Object) throw new InputException($"sample {result.Count} must be an object");
                var values = new Dictionary<string, BigInteger>(StringComparer.Ordinal);
                foreach (var signal in sample.EnumerateObject())
                {
                    if (!SignalName(signal.Name)) throw new InputException($"invalid signal name: {signal.Name}");
                    var value = signal.Value;
                    BigInteger integer;
                    if (value.ValueKind == JsonValueKind.True) integer = BigInteger.One;
                    else if (value.ValueKind == JsonValueKind.False) integer = BigInteger.Zero;
                    else if (value.ValueKind != JsonValueKind.Number || !BigInteger.TryParse(value.GetRawText(),
                        NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out integer))
                        throw new InputException($"sample {result.Count}, signal {signal.Name}: expected boolean or integer");
                    if (!values.TryAdd(signal.Name, integer)) throw new InputException($"duplicate signal: {signal.Name}");
                }
                result.Add(new(values));
            }
            return new(contract, result.AsReadOnly());
        }
        catch (JsonException error) { throw new InputException($"invalid JSON: {error.Message}"); }
    }

    private static bool SignalName(string name) => name.Length > 0 && char.IsAsciiLetter(name[0])
        && name.All(c => char.IsAsciiLetterOrDigit(c) || c == '_')
        && name is not "True" and not "False" and not "and" and not "or" and not "not" and not "at_trigger";
    private static int Delay(JsonElement element) => element.ValueKind == JsonValueKind.Number
        && element.TryGetInt32(out int value) && value >= 1 ? value
        : throw new InputException("delay must be a positive 32-bit integer");
    private static string Text(JsonElement element) => element.ValueKind == JsonValueKind.String
        ? element.GetString()! : throw new InputException("expected string");
    private static void Array(JsonElement element, string context)
    {
        if (element.ValueKind != JsonValueKind.Array) throw new InputException($"{context} must be an array");
    }
    private static void Object(JsonElement element, string context, string[] allowed, string[]? required = null)
    {
        if (element.ValueKind != JsonValueKind.Object) throw new InputException($"{context} must be an object");
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var property in element.EnumerateObject())
        {
            if (!seen.Add(property.Name)) throw new InputException($"duplicate {context} field: {property.Name}");
            if (!allowed.Contains(property.Name, StringComparer.Ordinal)) throw new InputException($"unknown {context} field: {property.Name}");
        }
        foreach (var name in required ?? [])
            if (!seen.Contains(name)) throw new InputException($"missing {context} field: {name}");
    }
    private static IEnumerable<PredicateClause> Predicates(JsonElement spec, string role)
    {
        if (!spec.TryGetProperty(role, out var clauses)) return [];
        if (clauses.ValueKind != JsonValueKind.Object) throw new InputException($"{role} must map labels to expressions");
        return clauses.EnumerateObject().Select(p => new PredicateClause(p.Name, Expression.Predicate(Text(p.Value)))).ToArray();
    }
}
