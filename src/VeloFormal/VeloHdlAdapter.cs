using VeloFormal.Language;
using VeloHdl;

namespace VeloFormal;

/// <summary>
/// Converts a provenance-declared VeloHdl envelope into VeloFormal's explicit
/// transition-model input. Formal semantics remain owned by VeloFormal.
/// </summary>
public static class VeloHdlAdapter
{
    /// <summary>Parses the structural payload and preserves its provenance into solver evidence.</summary>
    public static TransitionModel ToTransitionModel(StructuralDesign design)
    {
        ArgumentNullException.ThrowIfNull(design);
        var parsed = TransitionModel.Parse(design.ModelJson(), $"velohdl:{design.Provenance.SourceSha256}");
        return new TransitionModel(parsed.Module, parsed.Clock, parsed.Signals, parsed.Initial, parsed.Next,
            parsed.Parameters, design.Provenance);
    }
}
