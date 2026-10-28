using System.Text.Json;
using System.Security.Cryptography;
using System.Text;

namespace VeloHdl;

/// <summary>
/// Immutable provenance recorded with a normalized structural HDL design.
/// Hashes are lowercase SHA-256 hexadecimal strings. They become verified
/// bindings only when the corresponding source or IR bytes are available.
/// </summary>
public sealed record StructuralProvenance(string SourceSha256, string Frontend, string FrontendRevision,
    string NormalizedIrSha256, string Adapter, string AdapterVersion)
{
    /// <summary>Stable identity of the first VeloHdl lowering adapter.</summary>
    public const string DefaultAdapter = "VeloHdl.VeloSimNormalizedTranslator";
    /// <summary>Semantic version of the accepted normalized-UHDM subset.</summary>
    public const string DefaultAdapterVersion = "1";
    /// <summary>Creates reproducible SHA-256 provenance from decoded UTF-8 source and normalized-IR text.</summary>
    public static StructuralProvenance Create(string source, string frontend, string frontendRevision, string normalizedIr)
    {
        ArgumentNullException.ThrowIfNull(source);
        ArgumentException.ThrowIfNullOrWhiteSpace(frontend);
        ArgumentException.ThrowIfNullOrWhiteSpace(frontendRevision);
        ArgumentNullException.ThrowIfNull(normalizedIr);
        return new(CalculateSha256(source), frontend, frontendRevision, CalculateSha256(normalizedIr), DefaultAdapter, DefaultAdapterVersion);
    }

    public static string CalculateSha256(string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(value))).ToLowerInvariant();
    }
}

/// <summary>
/// Versioned envelope around a transition-model payload emitted by an HDL frontend.
/// VeloHdl validates the declared provenance shape and envelope shape only;
/// language- and solver-specific interpretation belongs to its consumers.
/// </summary>
public sealed class StructuralDesign
{
    /// <summary>Current on-disk envelope identifier.</summary>
    public const string Format = "velohdl.structural.v1";

    /// <summary>Identifies the source and frontend artifacts that produced this design.</summary>
    public StructuralProvenance Provenance { get; }

    /// <summary>JSON payload to be interpreted by a downstream transition-system consumer.</summary>
    public JsonElement Model { get; }

    private StructuralDesign(StructuralProvenance provenance, JsonElement model)
    {
        Provenance = provenance;
        Model = model;
    }

    /// <summary>Parses a strict, versioned VeloHdl structural-design envelope.</summary>
    /// <exception cref="InvalidDataException">Thrown when required provenance or envelope fields are absent or malformed.</exception>
    public static StructuralDesign Parse(string json)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(json);
        try
        {
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            Object(root, "format", "provenance", "model");
            if (Text(root, "format") != Format) throw new InvalidDataException("Unsupported VeloHdl structural format.");
            var provenance = root.GetProperty("provenance");
            Object(provenance, "source_sha256", "frontend", "frontend_revision", "normalized_ir_sha256", "adapter", "adapter_version");
            string source = Text(provenance, "source_sha256");
            string frontend = Text(provenance, "frontend");
            string revision = Text(provenance, "frontend_revision");
            string normalized = Text(provenance, "normalized_ir_sha256");
            string adapter = Text(provenance, "adapter");
            string adapterVersion = Text(provenance, "adapter_version");
            Hash(source, "source_sha256");
            Hash(normalized, "normalized_ir_sha256");
            if (string.IsNullOrWhiteSpace(frontend) || string.IsNullOrWhiteSpace(revision) ||
                string.IsNullOrWhiteSpace(adapter) || string.IsNullOrWhiteSpace(adapterVersion))
                throw new InvalidDataException("Frontend and adapter provenance must be nonempty.");
            var model = root.GetProperty("model");
            if (model.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Structural model must be an object.");
            return new(new(source, frontend, revision, normalized, adapter, adapterVersion), model.Clone());
        }
        catch (Exception error) when (error is JsonException or InvalidOperationException or FormatException or OverflowException)
        {
            throw new InvalidDataException("Malformed VeloHdl structural design: " + error.Message, error);
        }
    }

    /// <summary>Returns the preserved transition-model payload without altering its bytes' semantics.</summary>
    public string ModelJson() => Model.GetRawText();

    /// <summary>Serializes the strict envelope for handoff between frontend and formal tools.</summary>
    public string ToJson()
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream, new JsonWriterOptions { Indented = true }))
        {
            writer.WriteStartObject(); writer.WriteString("format", Format);
            writer.WritePropertyName("provenance"); writer.WriteStartObject();
            writer.WriteString("source_sha256", Provenance.SourceSha256); writer.WriteString("frontend", Provenance.Frontend);
            writer.WriteString("frontend_revision", Provenance.FrontendRevision); writer.WriteString("normalized_ir_sha256", Provenance.NormalizedIrSha256);
            writer.WriteString("adapter", Provenance.Adapter); writer.WriteString("adapter_version", Provenance.AdapterVersion);
            writer.WriteEndObject(); writer.WritePropertyName("model"); Model.WriteTo(writer); writer.WriteEndObject();
        }
        return Encoding.UTF8.GetString(stream.ToArray()) + "\n";
    }

    private static string Text(JsonElement value, string name)
    {
        var property = value.GetProperty(name);
        return property.ValueKind == JsonValueKind.String && property.GetString() is { } text
            ? text
            : throw new InvalidDataException($"'{name}' must be a string.");
    }

    private static void Object(JsonElement value, params string[] fields)
    {
        if (value.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Expected JSON object.");
        var properties = value.EnumerateObject().ToArray();
        var names = properties.Select(p => p.Name).ToHashSet(StringComparer.Ordinal);
        if (properties.Length != fields.Length || !names.SetEquals(fields)) throw new InvalidDataException("Unexpected, duplicate, or missing structural-design fields.");
    }

    private static void Hash(string value, string name)
    {
        if (value.Length != 64 || value.Any(c => c is not (>= '0' and <= '9') and not (>= 'a' and <= 'f')))
            throw new InvalidDataException($"'{name}' must be a lowercase SHA-256 hexadecimal string.");
    }
}
