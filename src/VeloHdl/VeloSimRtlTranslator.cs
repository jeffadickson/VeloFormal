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
            string next = string.Join(',', assignments.Select(assignment => $"{JsonSerializer.Serialize(assignment.Signal.Name)}:{JsonSerializer.Serialize(Expression(assignment.Value, signals, assignment.Signal.Width, assignment.Signal.Signed))}"));
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
    private sealed record Assignment(Signal Signal, JsonElement Value);

    /// <summary>Collects disjoint nonblocking updates evaluated from the same source frame.</summary>
    private static Assignment[] Assignments(JsonElement body, IReadOnlyList<Signal> signals, string clock)
    {
        JsonElement[] updates;
        string kind = Text(body, "kind");
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

    private static Signal ParseSignal(JsonElement signal)
    {
        Fields(signal, "name", "width", "direction", "two_state");
        string name = Text(signal, "name");
        int width = signal.GetProperty("width").GetInt32();
        string direction = Text(signal, "direction");
        if (width is < 1 or > 256 || direction is not ("input" or "output" or "internal") || !signal.GetProperty("two_state").GetBoolean())
            throw new InvalidDataException("Stage A requires bounded two-state signals.");
        return new(name, width, direction, Signed: false);
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
            Fields(value, "kind", "bits");
            string bits = Text(value, "bits");
            if (bits.Length != width || bits.Any(bit => bit is not ('0' or '1'))) throw new InvalidDataException("Stage A constants must be known destination-width bits.");
            return $"{width}'d{BinaryValue(bits)}";
        }
        if (kind == "add")
        {
            Fields(value, "kind", "args");
            var args = value.GetProperty("args").EnumerateArray().ToArray();
            if (args.Length != 2) throw new InvalidDataException("Stage A addition has arity two.");
            return $"({Expression(args[0], signals, width, signed)} + {Expression(args[1], signals, width, signed)})";
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
}
