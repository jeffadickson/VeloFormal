using System.Collections.ObjectModel;
using System.Numerics;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace VeloFormal.Language;
public enum SortKind { Boolean, Bits, Integer }
public readonly record struct LogicType(SortKind Kind, int Width = 0, bool Signed = false)
{
    public static LogicType Bool => new(SortKind.Boolean);
    public static LogicType Integer => new(SortKind.Integer);
    public static LogicType Bits(int width, bool signed = false) => new(SortKind.Bits, width, signed);
}
public sealed record SignalDeclaration(string Name, LogicType Type, bool IsState);
public sealed class TransitionModel
{
    public string Module { get; }
    public string Clock { get; }
    public IReadOnlyList<SignalDeclaration> Signals { get; }
    public ExprSyntax Initial { get; }
    public IReadOnlyDictionary<string, ExprSyntax> Next { get; }
    public IReadOnlyDictionary<string, BigInteger> Parameters { get; }
    public TransitionModel(string module, string clock, IEnumerable<SignalDeclaration> signals, ExprSyntax initial,
        IEnumerable<KeyValuePair<string, ExprSyntax>> next, IEnumerable<KeyValuePair<string, BigInteger>>? parameters = null)
    {
        Module = module; Clock = clock; Initial = initial;
        Signals = Array.AsReadOnly(signals.ToArray());
        if (!NameValid(module) || !NameValid(clock) || Signals.Count > 256) throw new InputException("invalid module, clock, or signal count");
        if (Signals.Any(s => !NameValid(s.Name) || s.Name is "true" or "false" ||
            s.Type.Kind is not SortKind.Boolean and not SortKind.Bits ||
            s.Type.Kind == SortKind.Boolean && (s.Type.Width != 0 || s.Type.Signed) || s.Type.Kind == SortKind.Bits && s.Type.Width is < 1 or > 256))
            throw new InputException("invalid signal declaration");
        if (Signals.Select(s => s.Name).Distinct(StringComparer.Ordinal).Count() != Signals.Count) throw new InputException("duplicate signal");
        Next = new ReadOnlyDictionary<string, ExprSyntax>(next.ToDictionary(p => p.Key, p => p.Value, StringComparer.Ordinal));
        Parameters = new ReadOnlyDictionary<string, BigInteger>((parameters ?? []).ToDictionary(p => p.Key, p => p.Value, StringComparer.Ordinal));
        if (Parameters.Count > 256 || Parameters.Any(p => !NameValid(p.Key) || p.Value.GetBitLength() > 4096) ||
            Parameters.Keys.Intersect(Signals.Select(s => s.Name), StringComparer.Ordinal).Any()) throw new InputException("invalid or shadowed parameter");
        if (!Next.Keys.Order(StringComparer.Ordinal).SequenceEqual(Signals.Where(s => s.IsState).Select(s => s.Name).Order(StringComparer.Ordinal)))
            throw new InputException("next must define each state exactly once and no inputs");
    }
    private static bool NameValid(string name) => name is not "true" and not "false" and not "at_trigger" and not "prev" and not "ite" and not "resize" and not "signed" and not "unsigned" && Regex.IsMatch(name, @"^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$");
    public static TransitionModel Parse(string json, string file = "<model>")
    {
        try
        {
            if (json.Length > 1048576) throw new InputException("model exceeds 1 MiB");
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            Fields(root, ["module", "clock", "signals", "init", "next", "parameters"], ["module", "clock", "signals", "init", "next"]);
            var signals = new List<SignalDeclaration>();
            var array = root.GetProperty("signals");
            if (array.ValueKind != JsonValueKind.Array) throw new InputException("signals must be an array");
            if (array.GetArrayLength() > 256) throw new InputException("signal limit exceeded");
            foreach (var signal in array.EnumerateArray())
            {
                Fields(signal, ["name", "type", "width", "kind"], ["name", "type", "kind"]);
                string type = Text(signal.GetProperty("type")), kind = Text(signal.GetProperty("kind"));
                if (type is not "bool" and not "unsigned" and not "signed" || kind is not "state" and not "input")
                    throw new InputException("signal type must be bool/unsigned/signed and kind state/input");
                LogicType sort;
                if (type == "bool")
                {
                    if (signal.TryGetProperty("width", out _)) throw new InputException("bool cannot have a width");
                    sort = LogicType.Bool;
                }
                else
                {
                    if (!signal.TryGetProperty("width", out var width) || width.ValueKind != JsonValueKind.Number || !width.TryGetInt32(out int size)) throw new InputException("bitvector width required");
                    sort = LogicType.Bits(size, type == "signed");
                }
                signals.Add(new(Text(signal.GetProperty("name")), sort, kind == "state"));
            }
            var next = Map(root.GetProperty("next")).Select(p =>
                new KeyValuePair<string, ExprSyntax>(p.Name, LanguageExpressions.Parse(Text(p.Value), file + ":next:" + p.Name))).ToArray();
            var parameters = new Dictionary<string, BigInteger>(StringComparer.Ordinal);
            if (root.TryGetProperty("parameters", out var values))
                foreach (var parameter in Map(values))
                {
                    if (parameters.Count >= 256) throw new InputException("parameter limit exceeded");
                    if (parameter.Value.ValueKind != JsonValueKind.Number || !BigInteger.TryParse(parameter.Value.GetRawText(),
                        System.Globalization.NumberStyles.AllowLeadingSign, System.Globalization.CultureInfo.InvariantCulture, out var value))
                        throw new InputException("parameters must be integers");
                    parameters.Add(parameter.Name, value);
                }
            return new(Text(root.GetProperty("module")), Text(root.GetProperty("clock")), signals,
                LanguageExpressions.Parse(Text(root.GetProperty("init")), file + ":init"), next, parameters);
        }
        catch (JsonException error) { throw new InputException("invalid model JSON: " + error.Message); }
    }
    private static string Text(JsonElement value) => value.ValueKind == JsonValueKind.String ? value.GetString()! : throw new InputException("expected model string");
    private static JsonProperty[] Map(JsonElement value)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InputException("expected model object");
        var properties = value.EnumerateObject().ToArray();
        if (properties.Select(p => p.Name).Distinct(StringComparer.Ordinal).Count() != properties.Length) throw new InputException("duplicate model field");
        return properties;
    }
    private static void Fields(JsonElement value, string[] allowed, string[] required)
    {
        var properties = Map(value);
        if (properties.Any(p => !allowed.Contains(p.Name, StringComparer.Ordinal)) ||
            required.Any(name => !properties.Any(p => p.Name == name))) throw new InputException("unknown or missing model field");
    }
}
