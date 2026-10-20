using System.Text.Json;
using VeloFormal;

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
                output.WriteLine("VeloFormal: --trace FILE (or - for stdin), --demo, --help");
                return 0;
            }
            try
            {
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
            catch (Exception exception) when (exception is InputException or IOException or UnauthorizedAccessException)
            {
                error.WriteLine($"veloformal: {exception.Message}");
                return 2;
            }
        }
    }
}
