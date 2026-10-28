using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace VeloFormal.Build;

/// <summary>Runs the prepared negative Stage-1 SymbiYosys fixture and retains reproducible execution evidence.</summary>
public static class YosysStage1
{
    private static readonly FixtureDefinition Negative = new("counter-bound.sby", "counter.sv", "FAIL");
    private static readonly FixtureDefinition Positive = new("signed-compare-pass.sby", "signed-compare.sv", "PASS");
    private static readonly FixtureDefinition DeterministicNegative = new("counter-init-bound.sby", "counter-init.sv", "FAIL");

    /// <summary>
    /// Runs the fixture's bounded SymbiYosys job. Its expected assertion failure
    /// returns zero only when the SBY status file reports <c>FAIL</c>.
    /// </summary>
    public static int Run(string root, TextWriter output, TextWriter error)
        => RunFixture(root, output, error, Negative);

    /// <summary>Runs the prepared positive signed Stage-1 fixture and requires SBY's bounded <c>PASS</c> verdict.</summary>
    public static int RunPositive(string root, TextWriter output, TextWriter error)
        => RunFixture(root, output, error, Positive);

    /// <summary>Runs the initialized, environment-constrained counterexample fixture for cross-tool witness comparison.</summary>
    public static int RunDeterministicNegative(string root, TextWriter output, TextWriter error)
        => RunFixture(root, output, error, DeterministicNegative);

    private static int RunFixture(string root, TextWriter output, TextWriter error, FixtureDefinition fixture)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(root);
        string fixtureDirectory = Path.Combine(root, "tests", "fixtures", "velohdl");
        string script = Path.Combine(fixtureDirectory, fixture.Script);
        string source = Path.Combine(fixtureDirectory, fixture.Source);
        if (!File.Exists(script) || !File.Exists(source))
        {
            error.WriteLine("Stage-1 fixture files are missing.");
            return 2;
        }

        string run = DateTime.UtcNow.ToString("yyyyMMdd-HHmmss-fffffff", System.Globalization.CultureInfo.InvariantCulture);
        string directory = Path.Combine(root, "artifacts", "yosys-stage1", run);
        Directory.CreateDirectory(directory);
        string executable = Environment.GetEnvironmentVariable("SBY") ?? "sby";
        int? exitCode = null;
        string stdout = string.Empty;
        string stderr = string.Empty;
        string? startError = null;
        try
        {
            var start = new ProcessStartInfo(executable) { WorkingDirectory = fixtureDirectory, UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            // An explicit portable-suite launcher must resolve sibling Yosys
            // executables and DLLs without requiring a machine-wide PATH change.
            YosysProbe.ConfigurePortableSuitePath(start, executable);
            start.ArgumentList.Add("-f");
            start.ArgumentList.Add("-d");
            start.ArgumentList.Add(Path.Combine(directory, "sby"));
            start.ArgumentList.Add(fixture.Script);
            using var process = Process.Start(start) ?? throw new IOException("SymbiYosys did not start.");
            Task<string> standardOutput = process.StandardOutput.ReadToEndAsync();
            Task<string> standardError = process.StandardError.ReadToEndAsync();
            if (!process.WaitForExit(60000))
            {
                process.Kill(entireProcessTree: true);
                process.WaitForExit();
                startError = "SymbiYosys timed out after 60 seconds.";
            }
            else exitCode = process.ExitCode;
            stdout = standardOutput.GetAwaiter().GetResult();
            stderr = standardError.GetAwaiter().GetResult();
        }
        catch (Exception exception) when (exception is System.ComponentModel.Win32Exception or IOException or ArgumentException)
        {
            startError = exception.Message;
        }

        File.WriteAllText(Path.Combine(directory, "stdout.txt"), stdout);
        File.WriteAllText(Path.Combine(directory, "stderr.txt"), stderr);
        string? statusPath = Path.Combine(directory, "sby", "status");
        string? rawStatus = File.Exists(statusPath) ? File.ReadAllText(statusPath).Trim() : null;
        // Recent SBY writes a leading verdict plus task counters (for example,
        // "FAIL 2 0"). The verdict token is the stable contract; retain the
        // complete line separately for forensic evidence.
        string? status = rawStatus?.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).FirstOrDefault();
        string artifact = Path.Combine(directory, "result.json");
        File.WriteAllText(artifact, JsonSerializer.Serialize(new
        {
            format = "veloformal.yosys-stage1.v1",
            fixture = fixture.Script,
            expected_status = fixture.ExpectedStatus,
            source_sha256 = HashFile(source),
            sby_sha256 = HashFile(script),
            sby = executable,
            exit_code = exitCode,
            status,
            raw_status = rawStatus,
            error = startError
        }, new JsonSerializerOptions { WriteIndented = true }) + "\n");

        if (startError is not null)
        {
            error.WriteLine($"Stage-1 SBY run unavailable: {startError}. Artifact: {artifact}");
            return 2;
        }
        if (status == fixture.ExpectedStatus)
        {
            output.WriteLine($"Stage-1 SBY fixture produced expected {fixture.ExpectedStatus}; artifact: {artifact}");
            return 0;
        }
        error.WriteLine($"Stage-1 SBY fixture status was '{status ?? "missing"}', expected {fixture.ExpectedStatus}; artifact: {artifact}");
        return 1;
    }

    private static string HashFile(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();

    private sealed record FixtureDefinition(string Script, string Source, string ExpectedStatus);
}
