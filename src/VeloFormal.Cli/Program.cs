using System.Text.Json;
using VeloFormal;
using VeloFormal.Language;

return Command.Run(args, Console.In, Console.Out, Console.Error);

namespace VeloFormal
{
    public static class Command
    {
        private static readonly JsonSerializerOptions JsonOptions = new()
        {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
            WriteIndented = true
        };
        public static int Run(string[] args, TextReader input, TextWriter output, TextWriter error)
        {
            if (args.Length == 0 || args is ["--help"] or ["-h"])
            {
                output.WriteLine("VeloFormal: --trace FILE (or - for stdin), --demo, --samples FILE --contract FILE, --vcd FILE --contract FILE --clock PATH --signal NAME=PATH, --parse-contract FILE, --format-contract FILE, --translate-velosim-normalized FILE --module NAME --source FILE --frontend-revision REVISION, --translate-velohdl-normalized FILE --module NAME --source FILE --frontend-revision REVISION, --verify-velohdl FILE --source FILE --normalized-ir FILE, --check-contract FILE --model FILE [--depth N] [--timeout-ms N] [--max-queries N], --help");
                return 0;
            }
            try
            {
                CliSchema.Validate(args);
                if (args.Contains("--format-contract", StringComparer.Ordinal))
                {
                    output.Write(ContractCommand.Format(args));
                    return 0;
                }
                if (args.Contains("--translate-velosim-normalized", StringComparer.Ordinal))
                {
                    output.Write(ContractCommand.TranslateVeloSimNormalized(args));
                    return 0;
                }
                if (args.Contains("--translate-velohdl-normalized", StringComparer.Ordinal))
                {
                    output.Write(ContractCommand.TranslateVeloHdlNormalized(args));
                    return 0;
                }
                if (args.Contains("--verify-velohdl", StringComparer.Ordinal))
                {
                    output.WriteLine(JsonSerializer.Serialize(ContractCommand.VerifyVeloHdl(args), JsonOptions));
                    return 0;
                }
                if (args.Contains("--parse-contract", StringComparer.Ordinal))
                {
                    output.WriteLine(JsonSerializer.Serialize(ContractCommand.Parse(args), JsonOptions));
                    return 0;
                }
                if (args.Contains("--check-contract", StringComparer.Ordinal))
                {
                    var solverReport = ContractCommand.Check(args);
                    output.WriteLine(JsonSerializer.Serialize(solverReport, JsonOptions));
                    return solverReport.Status == "bounded-pass" ? 0 : solverReport.Status == "unknown" ? 3 : 1;
                }
                if (args.Contains("--vcd", StringComparer.Ordinal))
                {
                    var wave = WaveformCommand.Read(args, input);
                    var waveReport = TraceChecker.Check(wave.Contract, wave.Samples);
                    output.WriteLine(JsonSerializer.Serialize(waveReport, JsonOptions));
                    return waveReport.Status == "trace-pass" ? 0 : 1;
                }
                if (args.Contains("--samples", StringComparer.Ordinal))
                {
                    var streamReport = StreamCommand.Check(args, input);
                    output.WriteLine(JsonSerializer.Serialize(streamReport, JsonOptions));
                    return streamReport.Status == "trace-pass" ? 0 : 1;
                }
                string json;
                if (args is ["--demo"])
                    json = """{"contract":{"invariants":{"nonnegative":"count >= 0"}},"samples":[{"count":0},{"count":1}]}""";
                else if (args is ["--trace", var path])
                    json = path == "-" ? input.ReadToEnd() : File.ReadAllText(path);
                else throw new InputException("expected --trace FILE, --demo, or --help");
                var document = TraceDocument.Parse(json);
                var report = TraceChecker.Check(document.Contract, document.Samples);
                output.WriteLine(JsonSerializer.Serialize(report, JsonOptions));
                return report.Status == "trace-pass" ? 0 : 1;
            }
            catch (Exception exception) when (exception is InputException or LanguageException or IOException or UnauthorizedAccessException or ArgumentException or InvalidDataException or System.Text.Json.JsonException)
            {
                error.WriteLine($"veloformal: {exception.Message}");
                return 2;
            }
        }
    }
}
