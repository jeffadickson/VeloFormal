using System.Diagnostics;

// All development orchestration lives in this C# executable; no shell scripts.
var root = Directory.GetCurrentDirectory();
while (!File.Exists(Path.Combine(root, "VeloFormal.sln")))
    root = Directory.GetParent(root)?.FullName ?? throw new InvalidOperationException("run inside the VeloFormal checkout");
if (args.Length > 1 || (args.Length == 1 && args[0] is not "test" and not "coverage"))
{
    Console.Error.WriteLine("usage: dotnet run --project tools/VeloFormal.Build -- [test|coverage]");
    return 2;
}
bool coverage = args is ["coverage"];
var run = DateTime.UtcNow.ToString("yyyyMMdd-HHmmss-fffffff", System.Globalization.CultureInfo.InvariantCulture);
var results = Path.Combine(root, "artifacts", "tests", run);
int Run(params string[] arguments)
{
    var start = new ProcessStartInfo("dotnet") { WorkingDirectory = root, UseShellExecute = false };
    foreach (var argument in arguments) start.ArgumentList.Add(argument);
    Console.WriteLine("dotnet " + string.Join(" ", arguments));
    using var process = Process.Start(start) ?? throw new IOException("could not start dotnet");
    process.WaitForExit();
    return process.ExitCode;
}
int code = Run("build", "VeloFormal.sln", "-c", "Release");
if (code != 0) return code;
var test = new List<string> { "test", "tests/VeloFormal.Tests", "-c", "Release", "--no-build",
    "--results-directory", results, "--logger", "trx" };
if (coverage) test.AddRange(["--collect:XPlat Code Coverage", "--settings", "coverage.runsettings"]);
code = Run(test.ToArray());
if (code != 0 || !coverage) return code;
code = Run("tool", "restore");
if (code != 0) return code;
return Run("tool", "run", "reportgenerator", $"-reports:{results}/**/coverage.cobertura.xml",
    $"-targetdir:{root}/artifacts/coverage/{run}", "-reporttypes:Html;TextSummary;Cobertura");
