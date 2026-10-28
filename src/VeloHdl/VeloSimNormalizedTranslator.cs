using System.Text.Json;
using System.Numerics;
using System.Globalization;

namespace VeloHdl;

/// <summary>
/// Lowers VeloSim's current strict normalized UHDM format and VeloHdl's expanded
/// fixture grammar into a VeloHdl envelope. The VeloSim v1 producer is accepted
/// only for its reset/increment shape; richer expressions use the separately
/// versioned <c>velohdl.normalized.v1</c> fixture contract until VeloSim emits it.
/// </summary>
public static class VeloSimNormalizedTranslator
{
    /// <summary>Converts a supported VeloSim normalized UHDM document into a provenance-carrying structural design.</summary>
    public static StructuralDesign Translate(string normalizedJson, StructuralProvenance provenance, string module)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(module);
        if (provenance.NormalizedIrSha256 != StructuralProvenance.CalculateSha256(normalizedJson)) throw new InvalidDataException("Normalized IR hash does not match the supplied artifact.");
        try
        {
            using var document = JsonDocument.Parse(normalizedJson);
            var root = document.RootElement;
            Fields(root, "format", "signals", "processes");
            string normalizedFormat = Text(root, "format");
            if (normalizedFormat is not ("velosim.uhdm.normalized.v1" or "velohdl.normalized.v1"))
                throw new InvalidDataException("Unsupported normalized structural format.");
            var signals = root.GetProperty("signals").EnumerateArray().Select(ParseSignal).ToArray();
            if (signals.Select(signal => signal.Name).Distinct(StringComparer.Ordinal).Count() != signals.Length)
                throw new InvalidDataException("Normalized UHDM has duplicate signal names.");
            var processes = root.GetProperty("processes");
            if (processes.ValueKind != JsonValueKind.Array || processes.GetArrayLength() is < 1 or > 256)
                throw new InvalidDataException("One through 256 synchronous processes are required.");
            var stateNames = processes.EnumerateArray().Select(Destination).ToHashSet(StringComparer.Ordinal);
            var lowered = processes.EnumerateArray().Select(process => ParseSynchronousProcess(process, signals, stateNames)).ToArray();
            string clock = lowered[0].Clock;
            string reset = lowered[0].Reset;
            if (lowered.Any(process => process.Clock != clock || process.Reset != reset))
                throw new InvalidDataException("All processes must share the same clock and reset.");
            if (lowered.Select(process => process.State.Name).Distinct(StringComparer.Ordinal).Count() != lowered.Length)
                throw new InvalidDataException("Multiple processes may not write the same state signal.");
            if (normalizedFormat == "velosim.uhdm.normalized.v1" && !IsCurrentVeloSimV1Counter(processes, lowered))
                throw new InvalidDataException("The current VeloSim normalized v1 producer supports only one reset/increment process.");
            var states = lowered.Select(process => process.State.Name).ToHashSet(StringComparer.Ordinal);
            string modelSignals = string.Join(",", signals.Select(signal =>
            {
                string kind = states.Contains(signal.Name) ? "state" : "input";
                // A one-bit register is still an RTL bitvector state. Only scalar
                // non-state signals become Boolean controls in the current subset.
                string type = states.Contains(signal.Name) || signal.Width != 1 ? "unsigned" : "bool";
                string width = type == "bool" ? "" : $",\"width\":{signal.Width}";
                return $"{{\"name\":{JsonSerializer.Serialize(signal.Name)},\"type\":\"{type}\"{width},\"kind\":\"{kind}\"}}";
            }));
            string modelNext = string.Join(",", lowered.Select(process =>
                $"{JsonSerializer.Serialize(process.State.Name)}:{JsonSerializer.Serialize($"ite(!{reset}, {process.State.Width}'d0, {process.Next})")}"));
            string model = $"{{\"module\":{JsonSerializer.Serialize(module)},\"clock\":{JsonSerializer.Serialize(clock)},\"signals\":[{modelSignals}],\"init\":\"true\",\"next\":{{{modelNext}}}}}";
            string envelope = $"{{\"format\":\"{StructuralDesign.Format}\",\"provenance\":{{\"source_sha256\":{JsonSerializer.Serialize(provenance.SourceSha256)},\"frontend\":{JsonSerializer.Serialize(provenance.Frontend)},\"frontend_revision\":{JsonSerializer.Serialize(provenance.FrontendRevision)},\"normalized_ir_sha256\":{JsonSerializer.Serialize(provenance.NormalizedIrSha256)},\"adapter\":{JsonSerializer.Serialize(provenance.Adapter)},\"adapter_version\":{JsonSerializer.Serialize(provenance.AdapterVersion)}}},\"model\":{model}}}";
            return StructuralDesign.Parse(envelope);
        }
        catch (Exception error) when (error is JsonException or InvalidOperationException or FormatException or OverflowException)
        {
            throw new InvalidDataException("Malformed VeloSim normalized UHDM: " + error.Message, error);
        }
    }

    private static (string Name, int Width) ParseSignal(JsonElement signal)
    {
        Fields(signal, "name", "width");
        string name = Text(signal, "name");
        int width = signal.GetProperty("width").GetInt32();
        if (width is < 1 or > 256) throw new InvalidDataException("Signal width is outside VeloFormal bounds.");
        return (name, width);
    }

    private static (string Clock, string Reset, (string Name, int Width) State, string Next) ParseSynchronousProcess(JsonElement process, (string Name, int Width)[] signals, IReadOnlySet<string> stateNames)
    {
        Fields(process, "kind", "event", "body");
        if (Text(process, "kind") != "always_ff") throw new InvalidDataException("Only always_ff is supported.");
        var ev = process.GetProperty("event"); Fields(ev, "edge", "signal");
        if (Text(ev, "edge") != "posedge") throw new InvalidDataException("Only posedge clocks are supported.");
        string clock = Text(ev, "signal");
        var body = process.GetProperty("body"); Fields(body, "kind", "condition", "then", "else");
        if (Text(body, "kind") != "if") throw new InvalidDataException("Only reset-if bodies are supported.");
        var condition = body.GetProperty("condition"); Fields(condition, "kind", "operand");
        var operand = condition.GetProperty("operand"); Fields(operand, "kind", "name");
        if (Text(condition, "kind") != "not" || Text(operand, "kind") != "ref") throw new InvalidDataException("Only active-low reset conditions are supported.");
        string reset = Text(operand, "name");
        var then = body.GetProperty("then"); var otherwise = body.GetProperty("else"); Fields(then, "kind", "target", "value"); Fields(otherwise, "kind", "target", "value");
        if (Text(then, "kind") != "nba" || Text(otherwise, "kind") != "nba") throw new InvalidDataException("Only nonblocking updates are supported.");
        string target = Text(then, "target");
        if (target != Text(otherwise, "target")) throw new InvalidDataException("Reset branches must update the same state.");
        if (target == clock || target == reset) throw new InvalidDataException("Clock and reset signals cannot be state destinations.");
        var match = signals.SingleOrDefault(s => s.Name == target);
        if (string.IsNullOrEmpty(match.Name) || signals.SingleOrDefault(s => s.Name == clock).Width != 1 || signals.SingleOrDefault(s => s.Name == reset).Width != 1) throw new InvalidDataException("Clock, reset, or state signal is invalid.");
        Constant(then.GetProperty("value"), 0);
        return (clock, reset, match, Expression(otherwise.GetProperty("value"), signals, stateNames, match.Width));
    }

    private static void Constant(JsonElement value, int expected)
    {
        Fields(value, "kind", "value");
        if (Text(value, "kind") != "const" || value.GetProperty("value").GetInt32() != expected)
            throw new InvalidDataException("Unsupported counter constant.");
    }

    private static string Destination(JsonElement process)
    {
        if (process.ValueKind != JsonValueKind.Object || !process.TryGetProperty("body", out var body) ||
            body.ValueKind != JsonValueKind.Object || !body.TryGetProperty("then", out var then) ||
            then.ValueKind != JsonValueKind.Object || !then.TryGetProperty("target", out var target) ||
            target.ValueKind != JsonValueKind.String || target.GetString() is not { } name)
            throw new InvalidDataException("Unsupported normalized UHDM shape.");
        return name;
    }

    private static bool IsCurrentVeloSimV1Counter(JsonElement processes, (string Clock, string Reset, (string Name, int Width) State, string Next)[] lowered)
    {
        if (lowered.Length != 1) return false;
        var value = processes[0].GetProperty("body").GetProperty("else").GetProperty("value");
        if (value.ValueKind != JsonValueKind.Object || Text(value, "kind") != "add") return false;
        var left = value.GetProperty("lhs");
        var right = value.GetProperty("rhs");
        return left.ValueKind == JsonValueKind.Object && right.ValueKind == JsonValueKind.Object &&
            Text(left, "kind") == "ref" && Text(left, "name") == lowered[0].State.Name &&
            Text(right, "kind") == "const" && right.GetProperty("value").ValueKind == JsonValueKind.Number &&
            right.GetProperty("value").TryGetInt32(out int increment) && increment == 1;
    }

    /// <summary>
    /// Renders the intentionally small normalized expression grammar into the
    /// VeloFormal expression language. Width-changing references are explicit,
    /// preventing accidental host-integer or implicit sign-extension semantics.
    /// </summary>
    private static string Expression(JsonElement value, (string Name, int Width)[] signals, IReadOnlySet<string> stateNames, int resultWidth)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Normalized expression must be an object.");
        string kind = Text(value, "kind");
        if (kind == "const")
        {
            Fields(value, "kind", "value");
            var literal = value.GetProperty("value");
            if (literal.ValueKind != JsonValueKind.Number || !BigInteger.TryParse(literal.GetRawText(), NumberStyles.None,
                CultureInfo.InvariantCulture, out var constant) || constant.Sign < 0 || constant.GetBitLength() > resultWidth)
                throw new InvalidDataException("Constant does not fit the destination width.");
            return $"{resultWidth}'d{constant}";
        }
        if (kind == "ref")
        {
            Fields(value, "kind", "name");
            string name = Text(value, "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == name);
            if (string.IsNullOrEmpty(signal.Name)) throw new InvalidDataException("Expression references an unknown signal.");
            if (signal.Width == 1 && resultWidth != 1) throw new InvalidDataException("Boolean signal cannot be used as an arithmetic value.");
            return signal.Width == resultWidth ? name : $"resize({name}, {resultWidth})";
        }
        if (kind == "not")
        {
            Fields(value, "kind", "operand");
            return $"~({Expression(value.GetProperty("operand"), signals, stateNames, resultWidth)})";
        }
        if (kind is "add" or "sub" or "mul" or "and" or "or" or "xor" or "shl" or "shr")
        {
            Fields(value, "kind", "lhs", "rhs");
            string op = kind switch
            {
                "add" => "+", "sub" => "-", "mul" => "*", "and" => "&", "or" => "|", "xor" => "^",
                "shl" => "<<", "shr" => ">>", _ => throw new InvalidOperationException()
            };
            return $"({Expression(value.GetProperty("lhs"), signals, stateNames, resultWidth)} {op} {Expression(value.GetProperty("rhs"), signals, stateNames, resultWidth)})";
        }
        if (kind == "ite")
        {
            Fields(value, "kind", "condition", "then", "else");
            string condition = Condition(value.GetProperty("condition"), signals, stateNames);
            return $"ite({condition}, {Expression(value.GetProperty("then"), signals, stateNames, resultWidth)}, {Expression(value.GetProperty("else"), signals, stateNames, resultWidth)})";
        }
        throw new InvalidDataException("Unsupported normalized expression kind.");
    }

    /// <summary>
    /// Lowers the Boolean control grammar separately from bitvector expressions.
    /// Comparisons require an explicit width so constants have no hidden host-width
    /// interpretation, matching the width-explicit rule for state expressions.
    /// </summary>
    private static string Condition(JsonElement value, (string Name, int Width)[] signals, IReadOnlySet<string> stateNames)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Normalized condition must be an object.");
        string kind = Text(value, "kind");
        if (kind == "ref")
        {
            Fields(value, "kind", "name");
            string name = Text(value, "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == name);
            if (string.IsNullOrEmpty(signal.Name) || signal.Width != 1 || stateNames.Contains(name))
                throw new InvalidDataException("Condition references must be scalar input signals.");
            return name;
        }
        if (kind == "not")
        {
            Fields(value, "kind", "operand");
            return $"!({Condition(value.GetProperty("operand"), signals, stateNames)})";
        }
        if (kind is "and" or "or")
        {
            Fields(value, "kind", "lhs", "rhs");
            string op = kind == "and" ? "&&" : "||";
            return $"({Condition(value.GetProperty("lhs"), signals, stateNames)} {op} {Condition(value.GetProperty("rhs"), signals, stateNames)})";
        }
        if (kind is "eq" or "ne" or "lt" or "le" or "gt" or "ge")
        {
            Fields(value, "kind", "width", "lhs", "rhs");
            var width = value.GetProperty("width");
            if (width.ValueKind != JsonValueKind.Number || !width.TryGetInt32(out int bits) || bits is < 1 or > 256)
                throw new InvalidDataException("Comparison width must be from 1 through 256.");
            string op = kind switch { "eq" => "==", "ne" => "!=", "lt" => "<", "le" => "<=", "gt" => ">", "ge" => ">=", _ => throw new InvalidOperationException() };
            return $"({ComparisonOperand(value.GetProperty("lhs"), signals, stateNames, bits)} {op} {ComparisonOperand(value.GetProperty("rhs"), signals, stateNames, bits)})";
        }
        throw new InvalidDataException("Unsupported normalized condition kind.");
    }

    private static string ComparisonOperand(JsonElement value, (string Name, int Width)[] signals, IReadOnlySet<string> stateNames, int width)
    {
        if (value.ValueKind == JsonValueKind.Object && Text(value, "kind") == "ref")
        {
            Fields(value, "kind", "name");
            string name = Text(value, "name");
            var signal = signals.SingleOrDefault(candidate => candidate.Name == name);
            if (string.IsNullOrEmpty(signal.Name)) throw new InvalidDataException("Expression references an unknown signal.");
            if (signal.Width == 1 && !stateNames.Contains(name))
            {
                if (width != 1) throw new InvalidDataException("Boolean controls can only be compared at width one.");
                return $"ite({name}, 1'd1, 1'd0)";
            }
        }
        return Expression(value, signals, stateNames, width);
    }

    private static void Fields(JsonElement value, params string[] names)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Unsupported normalized UHDM shape.");
        var properties = value.EnumerateObject().ToArray();
        if (properties.Length != names.Length || !properties.Select(p => p.Name).ToHashSet(StringComparer.Ordinal).SetEquals(names)) throw new InvalidDataException("Unsupported normalized UHDM shape.");
    }

    private static string Text(JsonElement value, string name)
    {
        var property = value.GetProperty(name);
        return property.ValueKind == JsonValueKind.String && property.GetString() is { } text
            ? text
            : throw new InvalidDataException($"'{name}' must be a string.");
    }
}
