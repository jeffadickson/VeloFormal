using FsCheck;
using FsCheck.Xunit;
using VeloFormal.Language;

namespace VeloFormal.Tests;

/// <summary>Property-based invariants for canonical parser output with reproducible FsCheck input.</summary>
public sealed class PropertyTests
{
    /// <summary>
    /// A generated arithmetic chain must preserve its canonical formatting under a second parse/format pass.
    /// The input is intentionally bounded to the documented AST depth and each failing case is reproducible
    /// through the fixed FsCheck replay seed.
    /// </summary>
[Property(MaxTest = 100, Replay = "123456,789013")]
    public bool FormattingArithmeticChainsIsIdempotent(NonEmptyArray<byte> terms)
    {
        string expression = string.Join(" + ", terms.Get.Take(32).Select(value => (value % 16).ToString()));
        string source = $"contract Generated for m\n    clock posedge clk\n    invariant safe: count + {expression} >= count\nend";
        string once = ContractFormatter.Format(ContractParser.Parse(source, "generated.vfc"));
        string twice = ContractFormatter.Format(ContractParser.Parse(once, "formatted.vfc"));
        return once == twice;
    }
}
