using System.Diagnostics;
using VeloFormal;

// All development orchestration lives in this C# executable; no shell scripts.
var root = Directory.GetCurrentDirectory();
while (!File.Exists(Path.Combine(root, "VeloFormal.sln")))
    root = Directory.GetParent(root)?.FullName ?? throw new InvalidOperationException("run inside the VeloFormal checkout");
if (args.Length > 1 || (args.Length == 1 && args[0] is not "test" and not "coverage" and not "rtl"))
{
    Console.Error.WriteLine("usage: dotnet run --project tools/VeloFormal.Build -- [test|coverage|rtl]");
    return 2;
}
bool coverage = args is ["coverage"];
if (args is ["rtl"])
{
    string directory = Path.Combine(root, "artifacts", "rtl", Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(directory);
    int Tool(string executable, params string[] arguments)
    {
        var start = new ProcessStartInfo(executable) { WorkingDirectory = directory, UseShellExecute = false };
        // Windows Icarus backends load DLLs from the compiler's bin directory.
        if (Path.IsPathRooted(executable))
            start.Environment["PATH"] = Path.GetDirectoryName(executable) + Path.PathSeparator + Environment.GetEnvironmentVariable("PATH");
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        Console.WriteLine(executable + " " + string.Join(" ", arguments));
        using var process = Process.Start(start) ?? throw new IOException($"could not start {executable}");
        if (!process.WaitForExit(60000))
        {
            process.Kill(entireProcessTree: true);
            throw new IOException($"{executable} timed out after 60 seconds");
        }
        if (process.ExitCode != 0) Console.Error.WriteLine($"{executable} exited with code {process.ExitCode}");
        return process.ExitCode;
    }
    try
    {
        string executable = Path.Combine(directory, "divider.vvp");
        var compile = new List<string> { "-g2012", "-s", "tb", "-o", executable,
            Path.Combine(root, "tests/fixtures/picorv32/simpleuart.v"),
            Path.Combine(root, "tests/fixtures/picorv32/divider_tb.v") };
        if (Environment.GetEnvironmentVariable("IVL_ROOT") is { Length: > 0 } ivl)
            compile.InsertRange(0, ["-B", ivl]);
        int result = Tool(Environment.GetEnvironmentVariable("IVERILOG") ?? "iverilog", compile.ToArray());
        if (result != 0) return result;
        result = Tool(Environment.GetEnvironmentVariable("VVP") ?? "vvp", executable);
        if (result != 0) return result;
        var samples = VcdReader.Read(File.ReadAllText(Path.Combine(directory, "divider.vcd")), "tb.clk",
            new Dictionary<string, string> { ["resetn"] = "tb.resetn", ["actual"] = "tb.actual", ["expected"] = "tb.expected" });
        var contract = new Contract(invariants: [new("divider_matches", s => s["actual"] == s["expected"])],
            reset: s => s["resetn"] == 0);
        var report = TraceChecker.Check(contract, samples);
        Console.WriteLine($"PicoRV32 divider: {report.Status}, {report.ActiveSamples} active samples; waveform: {directory}");
        return report.Status == "trace-pass" ? 0 : 1;
    }
    catch (Exception error) when (error is IOException or InputException or System.ComponentModel.Win32Exception)
    {
        Console.Error.WriteLine($"RTL check failed: {error.Message}. Set IVERILOG, VVP, and optionally IVL_ROOT.");
        return 2;
    }
}
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
