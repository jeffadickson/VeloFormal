using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace VeloFormal.Build;

/// <summary>Runs the prepared negative Stage-1 SymbiYosys fixture and retains reproducible execution evidence.</summary>
public static class YosysStage1
{
    private const string Fixture = "counter-bound.sby";
    private const string ExpectedStatus = "FAIL";

    /// <summary>
    /// Runs the fixture's bounded SymbiYosys job. Its expected assertion failure
    /// returns zero only when the SBY status file reports <c>FAIL</c>.
    /// </summary>
    public static int Run(string root, TextWriter output, TextWriter error)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(root);
        string fixtureDirectory = Path.Combine(root, "tests", "fixtures", "velohdl");
        string script = Path.Combine(fixtureDirectory, Fixture);
        string source = Path.Combine(fixtureDirectory, "counter.sv");
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
            start.ArgumentList.Add("-f");
            start.ArgumentList.Add("-d");
            start.ArgumentList.Add(Path.Combine(directory, "sby"));
            start.ArgumentList.Add(Fixture);
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
        string? status = File.Exists(statusPath) ? File.ReadAllText(statusPath).Trim() : null;
        string artifact = Path.Combine(directory, "result.json");
        File.WriteAllText(artifact, JsonSerializer.Serialize(new
        {
            format = "veloformal.yosys-stage1.v1",
            fixture = Fixture,
            expected_status = ExpectedStatus,
            source_sha256 = HashFile(source),
            sby_sha256 = HashFile(script),
            sby = executable,
            exit_code = exitCode,
            status,
            error = startError
        }, new JsonSerializerOptions { WriteIndented = true }) + "\n");

        if (startError is not null)
        {
            error.WriteLine($"Stage-1 SBY run unavailable: {startError}. Artifact: {artifact}");
            return 2;
        }
        if (status == ExpectedStatus)
        {
            output.WriteLine($"Stage-1 SBY fixture produced expected {ExpectedStatus}; artifact: {artifact}");
            return 0;
        }
        error.WriteLine($"Stage-1 SBY fixture status was '{status ?? "missing"}', expected {ExpectedStatus}; artifact: {artifact}");
        return 1;
    }

    private static string HashFile(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();
}
