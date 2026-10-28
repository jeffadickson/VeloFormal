using FsCheck;
using FsCheck.Xunit;
using System.Text.Json;
using System.Numerics;
using VeloHdl;
using VeloFormal.Language;
using VeloFormal.Solving;
using Xunit;

namespace VeloFormal.Tests;

/// <summary>Exercises the versioned HDL artifact boundary without requiring a VeloSim checkout.</summary>
public sealed class VeloHdlAdapterTests
{
    private const string Hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    [Fact]
    public void AdaptsStructuralModelAndCarriesDeclaredProvenanceToSolverEvidence()
    {
        const int iteration = 652;
        Assert.InRange(iteration, 651, 675);
        var design = StructuralDesign.Parse($$"""
            {"format":"velohdl.structural.v1","provenance":{"source_sha256":"{{Hash}}","frontend":"VeloSim","frontend_revision":"fa7cd2f","normalized_ir_sha256":"{{Hash}}","adapter":"VeloHdl.VeloSimNormalizedTranslator","adapter_version":"1"},"model":{{SemanticTests.ModelJson}}}
            """);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        Assert.Equal("m", model.Module);
        Assert.Equal("clk", model.Clock);
        Assert.Equal(Hash, design.Provenance.SourceSha256);
        var report = BoundedChecker.Check(SemanticBinder.Bind(ContractParser.Parse("contract C for m\n    clock posedge clk\n    invariant safe: true\nend"), model));
        Assert.Equal(Hash, report.InputProvenance?.SourceSha256);
        Assert.Equal(StructuralProvenance.DefaultAdapter, report.InputProvenance?.Adapter);
    }

    [Theory]
    [InlineData("ABCDEF")]
    [InlineData("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg")]
    public void RejectsUnpinnedOrMalformedProvenance(string hash)
    {
        string json = "{\"format\":\"velohdl.structural.v1\",\"provenance\":{\"source_sha256\":\"" + hash +
            "\",\"frontend\":\"VeloSim\",\"frontend_revision\":\"revision\",\"normalized_ir_sha256\":\"" + Hash +
            "\",\"adapter\":\"adapter\",\"adapter_version\":\"1\"},\"model\":{}}";
        Assert.Throws<InvalidDataException>(() => StructuralDesign.Parse(json));
    }

    [Fact]
    public void RejectsAmbiguousEnvelopeAndNormalizedSignalNames()
    {
        string duplicateEnvelope = "{\"format\":\"velohdl.structural.v1\",\"format\":\"velohdl.structural.v1\",\"provenance\":{\"source_sha256\":\"" + Hash + "\",\"frontend\":\"VeloSim\",\"frontend_revision\":\"revision\",\"normalized_ir_sha256\":\"" + Hash + "\",\"adapter\":\"adapter\",\"adapter_version\":\"1\"},\"model\":{}}";
        Assert.Throws<InvalidDataException>(() => StructuralDesign.Parse(duplicateEnvelope));
        Assert.Throws<InvalidDataException>(() => StructuralDesign.Parse("{\"format\":1,\"provenance\":{},\"model\":{}}"));
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string duplicateSignal = fixture.Replace("{\"name\":\"rst_n\",\"width\":1}", "{\"name\":\"clk\",\"width\":1}");
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(duplicateSignal, StructuralProvenance.Create("source", "VeloSim", "revision", duplicateSignal), "counter"));
    }

    [Fact]
    public void TranslatesVeloSimNormalizedCounterAndRejectsAlteredSemantics()
    {
        string fixture = Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json");
        string normalized = File.ReadAllText(fixture);
        var provenance = StructuralProvenance.Create("module counter;", "VeloSim", "fa7cd2f", normalized);
        Assert.NotEqual(provenance.SourceSha256, provenance.NormalizedIrSha256);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter"));
        Assert.Equal("counter", model.Module);
        Assert.Equal("clk", model.Clock);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant bounded: count < 2\nend");
        Assert.Equal("counterexample", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 3)).Status);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(normalized.Replace("\"value\":1", "\"value\":2"), provenance, "counter"));
    }

    [Fact]
    public void LowersSupportedMuxedArithmeticNextStateExpression()
    {
        const string normalized = """
            {"format":"velohdl.normalized.v1","signals":[{"name":"clk","width":1},{"name":"rst_n","width":1},{"name":"enable","width":1},{"name":"count","width":8}],"processes":[{"kind":"always_ff","event":{"edge":"posedge","signal":"clk"},"body":{"kind":"if","condition":{"kind":"not","operand":{"kind":"ref","name":"rst_n"}},"then":{"kind":"nba","target":"count","value":{"kind":"const","value":0}},"else":{"kind":"nba","target":"count","value":{"kind":"ite","condition":{"kind":"ref","name":"enable"},"then":{"kind":"add","lhs":{"kind":"ref","name":"count"},"rhs":{"kind":"const","value":1}},"else":{"kind":"xor","lhs":{"kind":"ref","name":"count"},"rhs":{"kind":"const","value":255}}}}}}]}
            """;
        var provenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter");
        var model = VeloHdlAdapter.ToTransitionModel(design);
        Assert.Equal(4, model.Signals.Count);
        using var emitted = JsonDocument.Parse(design.ModelJson());
        Assert.Equal("ite(!rst_n, 8'd0, ite(enable, (count + 8'd1), (count ^ 8'd255)))", emitted.RootElement.GetProperty("next").GetProperty("count").GetString());
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant range: count <= 8'd255\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 4)).Status);
    }

    [Fact]
    public void RejectsUnsupportedOrOutOfRangeNormalizedExpressions()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string multiplication = fixture.Replace("\"kind\":\"add\"", "\"kind\":\"mul\"", StringComparison.Ordinal);
        string overflow = fixture.Replace("\"value\":1", "\"value\":256", StringComparison.Ordinal);
        string reservedClockTarget = fixture.Replace("\"target\":\"count\"", "\"target\":\"clk\"", StringComparison.Ordinal);
        foreach (string candidate in new[] { multiplication, overflow, reservedClockTarget })
        {
            var provenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", candidate);
            Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(candidate, provenance, "counter"));
        }
    }

    [Fact]
    public void PreservesLargeConstantsWithinA256BitDestination()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string large = (BigInteger.One << 255).ToString(System.Globalization.CultureInfo.InvariantCulture);
        string normalized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("\"name\":\"count\",\"width\":8", "\"name\":\"count\",\"width\":256", StringComparison.Ordinal)
            .Replace("\"value\":1", "\"value\":" + large, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter");
        Assert.Contains(large, design.ModelJson(), StringComparison.Ordinal);
    }

    [Fact]
    public void LowersMultipleSingleClockRegistersAndRejectsMultiwriters()
    {
        const string processA = "{\"kind\":\"always_ff\",\"event\":{\"edge\":\"posedge\",\"signal\":\"clk\"},\"body\":{\"kind\":\"if\",\"condition\":{\"kind\":\"not\",\"operand\":{\"kind\":\"ref\",\"name\":\"rst_n\"}},\"then\":{\"kind\":\"nba\",\"target\":\"a\",\"value\":{\"kind\":\"const\",\"value\":0}},\"else\":{\"kind\":\"nba\",\"target\":\"a\",\"value\":{\"kind\":\"add\",\"lhs\":{\"kind\":\"ref\",\"name\":\"a\"},\"rhs\":{\"kind\":\"const\",\"value\":1}}}}}";
        string prefix = "{\"format\":\"velosim.uhdm.normalized.v1\",\"signals\":[{\"name\":\"clk\",\"width\":1},{\"name\":\"rst_n\",\"width\":1},{\"name\":\"a\",\"width\":2},{\"name\":\"b\",\"width\":2}],\"processes\":[";
        string normalized = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "two-registers.normalized.json"));
        var provenance = StructuralProvenance.Create("module two;", "VeloSim", "fixture", normalized);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "two"));
        Assert.Equal(["a", "b"], model.Signals.Where(signal => signal.IsState).Select(signal => signal.Name).Order());
        Assert.Equal(2, model.Next.Count);
        string multiwriter = prefix + processA + "," + processA + "]}";
        var badProvenance = StructuralProvenance.Create("module two;", "VeloSim", "fixture", multiwriter);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(multiwriter, badProvenance, "two"));
    }

    [Fact]
    public void CliEmitsReusableProvenanceCheckedEnvelope()
    {
        string directory = Path.Combine(Path.GetTempPath(), "velohdl-cli-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string normalizedPath = Path.Combine(directory, "counter.normalized.json");
            string sourcePath = Path.Combine(directory, "counter.sv");
            File.Copy(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"), normalizedPath);
            File.WriteAllText(sourcePath, "module counter; endmodule\n");
            using var output = new StringWriter(); using var error = new StringWriter();
            int code = Command.Run(["--translate-velosim-normalized", normalizedPath, "--module", "counter", "--source", sourcePath, "--frontend-revision", "fa7cd2f"], TextReader.Null, output, error);
            Assert.Equal(0, code); Assert.Empty(error.ToString());
            var design = StructuralDesign.Parse(output.ToString());
            Assert.Equal("VeloSim", design.Provenance.Frontend);
            Assert.Equal("counter", VeloHdlAdapter.ToTransitionModel(design).Module);
            string envelopePath = Path.Combine(directory, "counter.velohdl.json");
            File.WriteAllText(envelopePath, output.ToString());
            using var verifyOutput = new StringWriter(); using var verifyError = new StringWriter();
            int verified = Command.Run(["--verify-velohdl", envelopePath, "--source", sourcePath, "--normalized-ir", normalizedPath], TextReader.Null, verifyOutput, verifyError);
            Assert.Equal(0, verified); Assert.Empty(verifyError.ToString());
            using var verification = JsonDocument.Parse(verifyOutput.ToString());
            Assert.Equal("verified", verification.RootElement.GetProperty("status").GetString());
            File.AppendAllText(sourcePath, "// tampered\n");
            using var mismatchOutput = new StringWriter(); using var mismatchError = new StringWriter();
            Assert.Equal(2, Command.Run(["--verify-velohdl", envelopePath, "--source", sourcePath, "--normalized-ir", normalizedPath], TextReader.Null, mismatchOutput, mismatchError));
            Assert.StartsWith("veloformal: Source hash", mismatchError.ToString());
        }
        finally { Directory.Delete(directory, recursive: true); }
    }

    [Fact]
    public void CliTranslatesAndChecksMultiRegisterFixture()
    {
        string directory = Path.Combine(Path.GetTempPath(), "velohdl-multi-cli-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string normalizedPath = Path.Combine(directory, "two-registers.normalized.json");
            string sourcePath = Path.Combine(directory, "two.sv");
            string modelPath = Path.Combine(directory, "two.velohdl.json");
            string contractPath = Path.Combine(directory, "two.vfc");
            File.Copy(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "two-registers.normalized.json"), normalizedPath);
            File.WriteAllText(sourcePath, "module two; endmodule\n");
            using (var output = new StringWriter())
            using (var error = new StringWriter())
            {
                int code = Command.Run(["--translate-velosim-normalized", normalizedPath, "--module", "two", "--source", sourcePath, "--frontend-revision", "fixture"], TextReader.Null, output, error);
                Assert.Equal(0, code); Assert.Empty(error.ToString()); File.WriteAllText(modelPath, output.ToString());
            }
            File.WriteAllText(contractPath, "contract C for two\n    clock posedge clk\n    invariant ranges: a <= 2'd3 && b <= 2'd3\nend\n");
            using var checkOutput = new StringWriter(); using var checkError = new StringWriter();
            int check = Command.Run(["--check-contract", contractPath, "--model", modelPath, "--depth", "4"], TextReader.Null, checkOutput, checkError);
            Assert.Equal(0, check); Assert.Empty(checkError.ToString());
            using var report = JsonDocument.Parse(checkOutput.ToString());
            Assert.Equal("bounded-pass", report.RootElement.GetProperty("status").GetString());
            Assert.Equal("fixture", report.RootElement.GetProperty("input_provenance").GetProperty("frontend_revision").GetString());
        }
        finally { Directory.Delete(directory, recursive: true); }
    }

    /// <summary>Every supported VeloSim counter width must remain a matching VeloFormal bitvector width.</summary>
    [Property(MaxTest = 100, Replay = "260227,7919")]
    public bool PreservesSupportedCounterWidths(PositiveInt rawWidth)
    {
        int width = (rawWidth.Get - 1) % 256 + 1;
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string normalized = fixture.Replace("\"name\":\"count\",\"width\":8", "\"name\":\"count\",\"width\":" + width, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloSim", "fa7cd2f", normalized);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter"));
        return model.Signals.Single(signal => signal.Name == "count").Type.Width == width;
    }
}
