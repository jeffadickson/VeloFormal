using System.Numerics;
using System.Text.Json;

namespace VeloHdl;

/// <summary>
/// Lowers the deliberately small, two-state synchronous portion of VeloSim's
/// <c>velosim.rtl.v1</c> artifact into VeloHdl's provenance-carrying envelope.
/// Unsupported RTL is rejected rather than being given approximate formal meaning.
/// </summary>
public static class VeloSimRtlTranslator
{
    /// <summary>Identity recorded in envelopes emitted by this adapter.</summary>
    public const string Adapter = "VeloHdl.VeloSimRtlTranslator";

    /// <summary>Semantic subset version for the strict Stage A RTL bridge.</summary>
    public const string AdapterVersion = "1";

    /// <summary>
    /// Translates an implicit-clock, nonblocking, two-state VeloSim RTL body.
    /// The supplied provenance must bind the exact decoded UTF-8 IR text.
    /// </summary>
    public static StructuralDesign Translate(string rtlJson, StructuralProvenance provenance, string module)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(rtlJson);
        ArgumentNullException.ThrowIfNull(provenance);
        ArgumentException.ThrowIfNullOrWhiteSpace(module);
        if (provenance.NormalizedIrSha256 != StructuralProvenance.CalculateSha256(rtlJson))
            throw new InvalidDataException("RTL IR hash does not match the supplied artifact.");

        try
        {
            using var document = JsonDocument.Parse(rtlJson);
            var root = document.RootElement;
            Fields(root, "format", "signals", "memories", "continuous", "combinational", "clock", "initialization", "body");
            if (Text(root, "format") != "velosim.rtl.v1") throw new InvalidDataException("Unsupported VeloSim RTL format.");
            EmptyArray(root.GetProperty("memories"), "memories");
            EmptyArray(root.GetProperty("continuous"), "continuous");
            EmptyArray(root.GetProperty("combinational"), "combinational");
            EmptyBlock(root.GetProperty("initialization"), "initialization");

            var signals = root.GetProperty("signals").EnumerateArray().Select(ParseSignal).ToArray();
            if (signals.Length is < 1 or > 256 || signals.Select(static signal => signal.Name).Distinct(StringComparer.Ordinal).Count() != signals.Length)
                throw new InvalidDataException("RTL signals must be unique and bounded.");
            string clock = Text(root, "clock");
            var clockSignal = signals.SingleOrDefault(signal => signal.Name == clock);
            if (clockSignal is null || clockSignal.Width != 1 || clockSignal.Direction != "input")
                throw new InvalidDataException("Stage A requires a scalar input clock.");

            var assignments = Assignments(root.GetProperty("body"), signals, clock);
            var states = assignments.Select(static assignment => assignment.Signal.Name).ToHashSet(StringComparer.Ordinal);
            string modelSignals = string.Join(',', signals.Select(signal =>
            {
                string kind = states.Contains(signal.Name) ? "state" : "input";
                string type = states.Contains(signal.Name) || signal.Width != 1 ? signal.Signed ? "signed" : "unsigned" : "bool";
                return $"{{\"name\":{JsonSerializer.Serialize(signal.Name)},\"type\":\"{type}\"{(type == "bool" ? "" : $",\"width\":{signal.Width}")},\"kind\":\"{kind}\"}}";
            }));
            string next = string.Join(',', assignments.Select(assignment =>
            {
                string whenTrue = Expression(assignment.Value, signals, assignment.Signal.Width, assignment.Signal.Signed);
                string value = assignment.Condition is null ? whenTrue
                    : $"ite({assignment.Condition}, {whenTrue}, {Expression(assignment.ElseValue!.Value, signals, assignment.Signal.Width, assignment.Signal.Signed)})";
                return $"{JsonSerializer.Serialize(assignment.Signal.Name)}:{JsonSerializer.Serialize(value)}";
            }));
            string model = $"{{\"module\":{JsonSerializer.Serialize(module)},\"clock\":{JsonSerializer.Serialize(clock)},\"signals\":[{modelSignals}],\"init\":\"true\",\"next\":{{{next}}}}}";
            var emitted = new StructuralProvenance(provenance.SourceSha256, provenance.Frontend, provenance.FrontendRevision,
                provenance.NormalizedIrSha256, Adapter, AdapterVersion);
            string envelope = $"{{\"format\":\"{StructuralDesign.Format}\",\"provenance\":{{\"source_sha256\":{JsonSerializer.Serialize(emitted.SourceSha256)},\"frontend\":{JsonSerializer.Serialize(emitted.Frontend)},\"frontend_revision\":{JsonSerializer.Serialize(emitted.FrontendRevision)},\"normalized_ir_sha256\":{JsonSerializer.Serialize(emitted.NormalizedIrSha256)},\"adapter\":{JsonSerializer.Serialize(Adapter)},\"adapter_version\":\"{AdapterVersion}\"}},\"model\":{model}}}";
            return StructuralDesign.Parse(envelope);
        }
        catch (Exception error) when (error is JsonException or InvalidOperationException or FormatException or OverflowException)
        {
            throw new InvalidDataException("Malformed VeloSim RTL: " + error.Message, error);
        }
    }

    private sealed record Signal(string Name, int Width, string Direction, bool Signed);
    private sealed record Assignment(Signal Signal, JsonElement Value, string? Condition = null, JsonElement? ElseValue = null);

    /// <summary>Collects disjoint nonblocking updates evaluated from the same source frame.</summary>
    private static Assignment[] Assignments(JsonElement body, IReadOnlyList<Signal> signals, string clock)
    {
        JsonElement[] updates;
        string kind = Text(body, "kind");
        if (kind == "if") return [ConditionalAssignment(body, signals, clock)];
        if (kind == "block")
        {
            Fields(body, "kind", "items");
            var items = body.GetProperty("items").EnumerateArray().ToArray();
            if (items.Length != 1) throw new InvalidDataException("Stage A root blocks require exactly one synchronous statement.");
            return Assignments(items[0], signals, clock);
        }
        if (kind == "nba") updates = [body];
        else if (kind == "parallel")
        {
            Fields(body, "kind", "items");
            updates = body.GetProperty("items").EnumerateArray().ToArray();
            if (updates.Length is < 1 or > 256) throw new InvalidDataException("Stage A parallel body must contain one through 256 updates.");
        }
        else throw new InvalidDataException("Stage A requires an implicit-clock nonblocking body.");

        var result = new List<Assignment>(updates.Length);
        var destinations = new HashSet<string>(StringComparer.Ordinal);
        foreach (var update in updates)
        {
            Fields(update, "kind", "target", "value");
            if (Text(update, "kind") != "nba") throw new InvalidDataException("Stage A accepts only nonblocking updates.");
            var target = update.GetProperty("target");
            Fields(target, "kind", "name");
            if (Text(target, "kind") != "ref") throw new InvalidDataException("Stage A only assigns whole signal references.");
            string name = Text(target, "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == name);
            if (signal is null || name == clock || signal.Direction == "input" || !destinations.Add(name))
                throw new InvalidDataException("Stage A state destination is invalid or has multiple writers.");
            result.Add(new(signal, update.GetProperty("value").Clone()));
        }
        return [.. result];
    }

    /// <summary>Lowers a complete two-branch nonblocking assignment into an explicit formal mux.</summary>
    private static Assignment ConditionalAssignment(JsonElement statement, IReadOnlyList<Signal> signals, string clock)
    {
        Fields(statement, "kind", "condition", "then", "else");
        var whenTrue = SingleAssignment(statement.GetProperty("then"), signals, clock);
        var whenFalse = SingleAssignment(statement.GetProperty("else"), signals, clock);
        if (whenTrue.Signal.Name != whenFalse.Signal.Name)
            throw new InvalidDataException("Stage A if branches must update the same state word.");
        return new(whenTrue.Signal, whenTrue.Value, BooleanCondition(statement.GetProperty("condition"), signals), whenFalse.Value);
    }

    private static Assignment SingleAssignment(JsonElement update, IReadOnlyList<Signal> signals, string clock)
    {
        Fields(update, "kind", "target", "value");
        if (Text(update, "kind") != "nba") throw new InvalidDataException("Stage A branches require nonblocking updates.");
        var target = update.GetProperty("target");
        Fields(target, "kind", "name");
        if (Text(target, "kind") != "ref") throw new InvalidDataException("Stage A only assigns whole signal references.");
        string name = Text(target, "name");
        var signal = signals.SingleOrDefault(candidate => candidate.Name == name);
        if (signal is null || name == clock || signal.Direction == "input") throw new InvalidDataException("Stage A state destination is invalid.");
        return new(signal, update.GetProperty("value").Clone());
    }

    private static string BooleanCondition(JsonElement value, IReadOnlyList<Signal> signals)
    {
        string kind = Text(value, "kind");
        if (kind == "ref")
        {
            Fields(value, "kind", "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == Text(value, "name"));
            if (signal is null || signal.Width != 1 || signal.Signed || signal.Direction != "input")
                throw new InvalidDataException("Stage A condition must be an unsigned scalar input.");
            return signal.Name;
        }
        if (kind == "not")
        {
            Fields(value, "kind", "args");
            var args = value.GetProperty("args").EnumerateArray().ToArray();
            if (args.Length != 1) throw new InvalidDataException("Stage A logical negation has arity one.");
            return "!" + BooleanCondition(args[0], signals);
        }
        throw new InvalidDataException("Stage A condition must be a scalar input reference or its logical negation.");
    }

    private static Signal ParseSignal(JsonElement signal)
    {
        OptionalFields(signal, ["name", "width", "direction", "two_state"], ["signed"]);
        string name = Text(signal, "name");
        int width = signal.GetProperty("width").GetInt32();
        string direction = Text(signal, "direction");
        if (width is < 1 or > 256 || direction is not ("input" or "output" or "internal") || !signal.GetProperty("two_state").GetBoolean())
            throw new InvalidDataException("Stage A requires bounded two-state signals.");
        bool signed = signal.TryGetProperty("signed", out var sign) && sign.ValueKind == JsonValueKind.True
            ? true
            : signal.TryGetProperty("signed", out sign) && sign.ValueKind != JsonValueKind.False
                ? throw new InvalidDataException("RTL signal signedness must be Boolean.")
                : false;
        return new(name, width, direction, signed);
    }

    private static string Expression(JsonElement value, IReadOnlyList<Signal> signals, int width, bool signed)
    {
        string kind = Text(value, "kind");
        if (kind == "ref")
        {
            Fields(value, "kind", "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == Text(value, "name"));
            if (signal is null || signal.Width != width || signal.Signed != signed) throw new InvalidDataException("Stage A reference type mismatch.");
            return signal.Name;
        }
        if (kind == "const")
        {
            OptionalFields(value, ["kind", "bits"], ["signed"]);
            string bits = Text(value, "bits");
            if (bits.Length != width || bits.Any(bit => bit is not ('0' or '1'))) throw new InvalidDataException("Stage A constants must be known destination-width bits.");
            bool constantSigned = value.TryGetProperty("signed", out var sign) && sign.ValueKind == JsonValueKind.True
                ? true
                : value.TryGetProperty("signed", out sign) && sign.ValueKind != JsonValueKind.False
                    ? throw new InvalidDataException("RTL constant signedness must be Boolean.")
                    : false;
            if (constantSigned != signed) throw new InvalidDataException("Stage A arithmetic requires explicit matching signedness.");
            return $"{width}{(signed ? "'sd" : "'d")}{BinaryValue(bits)}";
        }
        if (kind is "add" or "sub" or "mul" or "bitand" or "bitor" or "bitxor" or "shl" or "shr" or "ashr")
        {
            Fields(value, "kind", "args");
            var args = value.GetProperty("args").EnumerateArray().ToArray();
            if (args.Length != 2) throw new InvalidDataException("Stage A binary expression has arity two.");
            string left = Expression(args[0], signals, width, signed);
            string right = Expression(args[1], signals, width, signed);
            if (kind == "shr" && signed)
                return $"signed(unsigned({left}) >> unsigned({right}))";
            string op = kind switch
            {
                "add" => "+", "sub" => "-", "mul" => "*", "bitand" => "&", "bitor" => "|", "bitxor" => "^",
                "shl" => "<<", "shr" or "ashr" => ">>", _ => throw new InvalidOperationException()
            };
            return $"({left} {op} {right})";
        }
        throw new InvalidDataException("Unsupported Stage A RTL expression.");
    }

    private static void EmptyArray(JsonElement value, string name)
    {
        if (value.ValueKind != JsonValueKind.Array || value.GetArrayLength() != 0) throw new InvalidDataException($"Stage A requires empty '{name}'.");
    }

    /// <summary>Converts a validated MSB-first binary literal without host-width truncation.</summary>
    private static BigInteger BinaryValue(string bits)
    {
        BigInteger value = BigInteger.Zero;
        foreach (char bit in bits) value = (value << 1) | (bit == '1' ? BigInteger.One : BigInteger.Zero);
        return value;
    }

    private static void EmptyBlock(JsonElement value, string name)
    {
        Fields(value, "kind", "items");
        if (Text(value, "kind") != "block") throw new InvalidDataException($"Stage A requires a block '{name}'.");
        EmptyArray(value.GetProperty("items"), name);
    }

    private static string Text(JsonElement value, string name) => value.GetProperty(name).ValueKind == JsonValueKind.String
        ? value.GetProperty(name).GetString()! : throw new InvalidDataException($"'{name}' must be a string.");

    private static void Fields(JsonElement value, params string[] names)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Expected a JSON object.");
        var properties = value.EnumerateObject().ToArray();
        if (properties.Length != names.Length || !properties.Select(static property => property.Name).ToHashSet(StringComparer.Ordinal).SetEquals(names))
            throw new InvalidDataException("Unexpected, duplicate, or missing RTL fields.");
    }

    private static void OptionalFields(JsonElement value, IReadOnlyCollection<string> required, IReadOnlyCollection<string> optional)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Expected a JSON object.");
        var properties = value.EnumerateObject().ToArray();
        var names = properties.Select(static property => property.Name).ToHashSet(StringComparer.Ordinal);
        var requiredNames = required.ToHashSet(StringComparer.Ordinal);
        if (!requiredNames.IsSubsetOf(names) || !names.IsSubsetOf(requiredNames.Concat(optional).ToHashSet(StringComparer.Ordinal)) ||
            properties.Length != names.Count)
            throw new InvalidDataException("Unexpected, duplicate, or missing RTL fields.");
    }
}
