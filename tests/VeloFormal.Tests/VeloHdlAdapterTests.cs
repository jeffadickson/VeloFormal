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
    [Fact]
    public void LowersCompleteProspectiveInitialStateAndRejectsPartialOrCurrentProducerState()
    {
        const string normalized = """
            {"format":"velohdl.normalized.v1","signals":[{"name":"clk","width":1},{"name":"rst_n","width":1},{"name":"count","width":8}],"processes":[{"kind":"always_ff","event":{"edge":"posedge","signal":"clk"},"body":{"kind":"if","condition":{"kind":"not","operand":{"kind":"ref","name":"rst_n"}},"then":{"kind":"nba","target":"count","value":{"kind":"const","value":0}},"else":{"kind":"nba","target":"count","value":{"kind":"add","lhs":{"kind":"ref","name":"count"},"rhs":{"kind":"const","value":1}}}}}],"initial":[{"target":"count","value":{"kind":"const","value":0}}]}
            """;
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter");
        using (var emitted = JsonDocument.Parse(design.ModelJson()))
            Assert.Equal("(count == 8'd0)", emitted.RootElement.GetProperty("init").GetString());
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant initial: count == 8'd0\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 0)).Status);

        string partial = normalized.Replace("[{\"target\":\"count\",\"value\":{\"kind\":\"const\",\"value\":0}}]", "[]", StringComparison.Ordinal);
        var partialProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", partial);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(partial, partialProvenance, "counter"));
        string current = normalized.Replace("velohdl.normalized.v1", "velosim.uhdm.normalized.v1", StringComparison.Ordinal);
        var currentProvenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", current);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(current, currentProvenance, "counter"));
    }

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
    public void LowersFixtureResetConstantsButKeepsCurrentVeloSimZeroResetOnly()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string initialized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("\"value\":0", "\"value\":7", StringComparison.Ordinal);
        var fixtureProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", initialized);
        var design = VeloSimNormalizedTranslator.Translate(initialized, fixtureProvenance, "counter");
        using (var lowered = JsonDocument.Parse(design.ModelJson()))
            Assert.Contains("ite(!rst_n, 8'd7", lowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);

        string signedInitialized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":true}", StringComparison.Ordinal)
            .Replace("\"value\":0", "\"value\":-1", StringComparison.Ordinal);
        var signedProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", signedInitialized);
        var signedDesign = VeloSimNormalizedTranslator.Translate(signedInitialized, signedProvenance, "counter");
        using (var signedLowered = JsonDocument.Parse(signedDesign.ModelJson()))
            Assert.Contains("ite(!rst_n, -(8'sd1)", signedLowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);

        string currentProducer = initialized.Replace("velohdl.normalized.v1", "velosim.uhdm.normalized.v1", StringComparison.Ordinal);
        var currentProvenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", currentProducer);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(currentProducer, currentProvenance, "counter"));
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
    public void LowersExplicitWidthComparisonConditions()
    {
        const string normalized = """
            {"format":"velohdl.normalized.v1","signals":[{"name":"clk","width":1},{"name":"rst_n","width":1},{"name":"count","width":8}],"processes":[{"kind":"always_ff","event":{"edge":"posedge","signal":"clk"},"body":{"kind":"if","condition":{"kind":"not","operand":{"kind":"ref","name":"rst_n"}},"then":{"kind":"nba","target":"count","value":{"kind":"const","value":0}},"else":{"kind":"nba","target":"count","value":{"kind":"ite","condition":{"kind":"lt","width":8,"lhs":{"kind":"ref","name":"count"},"rhs":{"kind":"const","value":3}},"then":{"kind":"add","lhs":{"kind":"ref","name":"count"},"rhs":{"kind":"const","value":1}},"else":{"kind":"const","value":0}}}}}]}
            """;
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter");
        using var lowered = JsonDocument.Parse(design.ModelJson());
        Assert.Contains("count < 8'd3", lowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 5)).Status);
        string missingWidth = normalized.Replace("\"kind\":\"lt\",\"width\":8", "\"kind\":\"lt\"", StringComparison.Ordinal);
        var badProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", missingWidth);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(missingWidth, badProvenance, "counter"));
    }

    [Theory]
    [InlineData("mul", "*", 2)]
    [InlineData("shl", "<<", 1)]
    [InlineData("shr", ">>", 1)]
    public void LowersWidthExplicitArithmeticOperators(string normalizedKind, string expectedOperator, int operand)
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string normalized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("\"kind\":\"add\"", "\"kind\":\"" + normalizedKind + "\"", StringComparison.Ordinal)
            .Replace("\"value\":1", "\"value\":" + operand, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter");
        using var document = JsonDocument.Parse(design.ModelJson());
        Assert.Contains("count " + expectedOperator + " 8'd" + operand, document.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);
    }

    [Fact]
    public void LowersExplicitSignedStateArithmeticWithoutImplicitMixedOperands()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string signed = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":true}", StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", signed);
        var design = VeloSimNormalizedTranslator.Translate(signed, provenance, "counter");
        var model = VeloHdlAdapter.ToTransitionModel(design);
        Assert.Equal(LogicType.Bits(8, signed: true), model.Signals.Single(signal => signal.Name == "count").Type);
        using (var lowered = JsonDocument.Parse(design.ModelJson()))
            Assert.Equal("ite(!rst_n, 8'sd0, (count + 8'sd1))", lowered.RootElement.GetProperty("next").GetProperty("count").GetString());
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);

        string mixed = signed.Replace("\"value\":1", "\"name\":\"rst_n\"", StringComparison.Ordinal)
            .Replace("\"kind\":\"const\",\"name\":\"rst_n\"", "\"kind\":\"ref\",\"name\":\"rst_n\"", StringComparison.Ordinal);
        var mixedProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", mixed);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(mixed, mixedProvenance, "counter"));

        string currentProducerWithSignedFlag = fixture.Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":false}", StringComparison.Ordinal);
        var currentProvenance = StructuralProvenance.Create("module counter;", "VeloSim", "fixture", currentProducerWithSignedFlag);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(currentProducerWithSignedFlag, currentProvenance, "counter"));
    }

    [Fact]
    public void LowersExplicitSignedComparisonConditions()
    {
        const string increment = "{\"kind\":\"add\",\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":1}}";
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string signed = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":true}", StringComparison.Ordinal)
            .Replace(increment, "{\"kind\":\"ite\",\"condition\":{\"kind\":\"lt\",\"width\":8,\"signed\":true,\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":0}},\"then\":{\"kind\":\"const\",\"value\":0},\"else\":" + increment + "}", StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", signed);
        var design = VeloSimNormalizedTranslator.Translate(signed, provenance, "counter");
        using (var lowered = JsonDocument.Parse(design.ModelJson()))
            Assert.Contains("count < 8'sd0", lowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);

        string malformed = signed.Replace("\"signed\":true,\"lhs\"", "\"signed\":\"yes\",\"lhs\"", StringComparison.Ordinal);
        var malformedProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", malformed);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(malformed, malformedProvenance, "counter"));
    }

    [Fact]
    public void PairedSignedStage1FixtureBindsItsRealSourceAndContract()
    {
        string normalized = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "signed-compare.normalized.json"));
        string source = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "signed-compare.sv"));
        string contractText = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "signed-compare.vfc"));
        var provenance = StructuralProvenance.Create(source, "VeloHdlFixture", "stage1-fixture", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "signed_compare");
        Assert.Equal(StructuralProvenance.CalculateSha256(source), design.Provenance.SourceSha256);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(ContractParser.Parse(contractText), model), new SolverOptions(Depth: 3)).Status);
    }

    [Fact]
    public void PairedSignedBoundFixtureProducesTheExpectedBoundedPass()
    {
        string fixtureDirectory = Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl");
        string source = File.ReadAllText(Path.Combine(fixtureDirectory, "signed-compare.sv"));
        string normalized = File.ReadAllText(Path.Combine(fixtureDirectory, "signed-compare.normalized.json"));
        string contractText = File.ReadAllText(Path.Combine(fixtureDirectory, "signed-compare-pass.vfc"));
        string sby = File.ReadAllText(Path.Combine(fixtureDirectory, "signed-compare-pass.sby"));
        Assert.Contains("assert (count <= 8'sd127)", source, StringComparison.Ordinal);
        Assert.Contains("mode bmc", sby, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create(source, "VeloHdlFixture", "stage1-fixture", normalized);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "signed_compare"));
        var report = BoundedChecker.Check(SemanticBinder.Bind(ContractParser.Parse(contractText), model), new SolverOptions(Depth: 3));
        Assert.Equal("bounded-pass", report.Status);
        Assert.Equal(StructuralProvenance.CalculateSha256(source), report.InputProvenance?.SourceSha256);
    }

    [Fact]
    public void PairedCounterBoundFixtureProducesTheExpectedBoundedCounterexample()
    {
        string fixtureDirectory = Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl");
        string source = File.ReadAllText(Path.Combine(fixtureDirectory, "counter.sv"));
        string normalized = File.ReadAllText(Path.Combine(fixtureDirectory, "counter.normalized.json"));
        string contractText = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-bound.vfc"));
        string sby = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-bound.sby"));
        Assert.Contains("assert (count < 8'd2)", source, StringComparison.Ordinal);
        Assert.Contains("mode bmc", sby, StringComparison.Ordinal);
        Assert.Contains("depth 3", sby, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create(source, "VeloSim", "stage1-fixture", normalized);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter"));
        var report = BoundedChecker.Check(SemanticBinder.Bind(ContractParser.Parse(contractText), model), new SolverOptions(Depth: 3));
        Assert.Equal("counterexample", report.Status);
        Assert.Equal(StructuralProvenance.CalculateSha256(source), report.InputProvenance?.SourceSha256);
    }

    [Fact]
    public void InitializedCounterFixtureProducesAConstrainedCounterexample()
    {
        string fixtureDirectory = Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl");
        string source = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-init.sv"));
        string normalized = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-init.normalized.json"));
        string contractText = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-init-bound.vfc"));
        string sby = File.ReadAllText(Path.Combine(fixtureDirectory, "counter-init-bound.sby"));
        Assert.Contains("initial count = 8'd0", source, StringComparison.Ordinal);
        Assert.Contains("assume (rst_n)", source, StringComparison.Ordinal);
        Assert.Contains("depth 4", sby, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create(source, "VeloHdlFixture", "stage1-fixture", normalized);
        var model = VeloHdlAdapter.ToTransitionModel(VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter_init"));
        var report = BoundedChecker.Check(SemanticBinder.Bind(ContractParser.Parse(contractText), model), new SolverOptions(Depth: 3));
        Assert.Equal("counterexample", report.Status);
        Assert.Equal("0", report.Counterexample![0].Values["count"]);
        Assert.Equal("1", report.Counterexample[1].Values["count"]);
        Assert.Equal("2", report.Counterexample[2].Values["count"]);
    }

    [Fact]
    public void LowersNegativeSignedConstantsWithinTheirMathematicalRange()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string signed = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":true}", StringComparison.Ordinal)
            .Replace("\"value\":1", "\"value\":-1", StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", signed);
        var design = VeloSimNormalizedTranslator.Translate(signed, provenance, "counter");
        using (var lowered = JsonDocument.Parse(design.ModelJson()))
            Assert.Contains("count + -(8'sd1)", lowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);

        string outOfRange = signed.Replace("\"value\":-1", "\"value\":-129", StringComparison.Ordinal);
        var outOfRangeProvenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", outOfRange);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(outOfRange, outOfRangeProvenance, "counter"));
    }

    [Fact]
    public void LowersFixedWidthUnaryNegationForSignedState()
    {
        const string increment = "{\"kind\":\"add\",\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":1}}";
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        string signed = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("{\"name\":\"count\",\"width\":8}", "{\"name\":\"count\",\"width\":8,\"signed\":true}", StringComparison.Ordinal)
            .Replace(increment, "{\"kind\":\"neg\",\"operand\":{\"kind\":\"ref\",\"name\":\"count\"}}", StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", signed);
        var design = VeloSimNormalizedTranslator.Translate(signed, provenance, "counter");
        using (var lowered = JsonDocument.Parse(design.ModelJson()))
            Assert.Contains("-(count)", lowered.RootElement.GetProperty("next").GetProperty("count").GetString(), StringComparison.Ordinal);
        var model = VeloHdlAdapter.ToTransitionModel(design);
        var contract = ContractParser.Parse("contract C for counter\n    clock posedge clk\n    invariant well_formed: true\nend");
        Assert.Equal("bounded-pass", BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: 2)).Status);
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
    public void RejectsBitvectorStateAsABooleanCondition()
    {
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        const string increment = "{\"kind\":\"add\",\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":1}}";
        const string badCondition = "{\"kind\":\"ite\",\"condition\":{\"kind\":\"ref\",\"name\":\"count\"},\"then\":{\"kind\":\"const\",\"value\":1},\"else\":{\"kind\":\"const\",\"value\":0}}";
        string normalized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("\"name\":\"count\",\"width\":8", "\"name\":\"count\",\"width\":1", StringComparison.Ordinal)
            .Replace(increment, badCondition, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module counter;", "VeloHdlFixture", "fixture", normalized);
        Assert.Throws<InvalidDataException>(() => VeloSimNormalizedTranslator.Translate(normalized, provenance, "counter"));
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
            File.Copy(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.sv"), sourcePath);
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
            using (var rejectedOutput = new StringWriter())
            using (var rejectedError = new StringWriter())
            {
                Assert.Equal(2, Command.Run(["--translate-velosim-normalized", normalizedPath, "--module", "two", "--source", sourcePath, "--frontend-revision", "fixture"], TextReader.Null, rejectedOutput, rejectedError));
                Assert.StartsWith("veloformal:", rejectedError.ToString());
            }
            using (var output = new StringWriter())
            using (var error = new StringWriter())
            {
                int code = Command.Run(["--translate-velohdl-normalized", normalizedPath, "--module", "two", "--source", sourcePath, "--frontend-revision", "fixture"], TextReader.Null, output, error);
                Assert.Equal(0, code); Assert.Empty(error.ToString()); File.WriteAllText(modelPath, output.ToString());
                Assert.Equal("VeloHdlFixture", StructuralDesign.Parse(output.ToString()).Provenance.Frontend);
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

    /// <summary>Comparison widths and constants must remain explicit over the supported bitvector range.</summary>
    [Property(MaxTest = 100, Replay = "281028,44017")]
    public bool PreservesRandomizedComparisonWidths(PositiveInt rawWidth, NonNegativeInt rawConstant)
    {
        int width = (rawWidth.Get - 1) % 256 + 1;
        BigInteger modulus = BigInteger.One << width;
        BigInteger constant = rawConstant.Get % modulus;
        string fixture = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "velohdl", "counter.normalized.json"));
        const string increment = "{\"kind\":\"add\",\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":1}}";
        string conditional = "{\"kind\":\"ite\",\"condition\":{\"kind\":\"le\",\"width\":" + width +
            ",\"lhs\":{\"kind\":\"ref\",\"name\":\"count\"},\"rhs\":{\"kind\":\"const\",\"value\":" + constant +
            "}},\"then\":{\"kind\":\"ref\",\"name\":\"count\"},\"else\":{\"kind\":\"const\",\"value\":0}}";
        string normalized = fixture.Replace("velosim.uhdm.normalized.v1", "velohdl.normalized.v1", StringComparison.Ordinal)
            .Replace("\"name\":\"count\",\"width\":8", "\"name\":\"count\",\"width\":" + width, StringComparison.Ordinal)
            .Replace(increment, conditional, StringComparison.Ordinal);
        var provenance = StructuralProvenance.Create("module count;", "VeloHdlFixture", "property", normalized);
        var design = VeloSimNormalizedTranslator.Translate(normalized, provenance, "count");
        using var document = JsonDocument.Parse(design.ModelJson());
        string next = document.RootElement.GetProperty("next").GetProperty("count").GetString()!;
        return next.Contains($"count <= {width}'d{constant}", StringComparison.Ordinal);
    }

    /// <summary>Arbitrary normalized text must either lower or fail with the public fail-closed diagnostic.</summary>
    [Property(MaxTest = 100, Replay = "281028,13057")]
    public bool RejectsRandomNormalizedTextWithoutUnexpectedExceptions(string? raw)
    {
        if (raw is null) return true;
        var provenance = StructuralProvenance.Create("fuzz", "Property", "1", raw);
        try
        {
            VeloSimNormalizedTranslator.Translate(raw, provenance, "fuzz");
        }
        catch (InvalidDataException)
        {
            // Expected for arbitrary input outside the strict normalized grammar.
        }
        return true;
    }
}
