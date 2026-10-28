using System.Diagnostics;
using System.Text.Json;

namespace VeloFormal.Build;

/// <summary>Captures the external tools required for future Yosys differential qualification.</summary>
public static class YosysProbe
{
    /// <summary>Writes a version/availability artifact. Missing tools are diagnostic evidence, not a build failure.</summary>
    public static int Run(string root, TextWriter output, TextWriter error)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(root);
        string run = DateTime.UtcNow.ToString("yyyyMMdd-HHmmss-fffffff", System.Globalization.CultureInfo.InvariantCulture);
        string directory = Path.Combine(root, "artifacts", "yosys-probe", run);
        Directory.CreateDirectory(directory);
        var tools = new[] { new Tool("yosys", "YOSYS", "-V"), new Tool("yosys-smtbmc", "YOSYS_SMTBMC", "-h"), new Tool("sby", "SBY", "--version"), new Tool("slang", "SLANG", "--version") }.Select(Probe).ToArray();
        string artifact = Path.Combine(directory, "toolchain.json");
        File.WriteAllText(artifact, JsonSerializer.Serialize(new { format = "veloformal.yosys-probe.v1", tools }, new JsonSerializerOptions { WriteIndented = true }) + "\n");
        int found = tools.Count(tool => tool.available);
        output.WriteLine($"Yosys differential toolchain: {found}/{tools.Length} available; artifact: {artifact}");
        foreach (var tool in tools.Where(tool => !tool.available)) error.WriteLine($"missing: {tool.name} (set {tool.environmentVariable})");
        return 0;
    }

    private static ProbeResult Probe(Tool tool)
    {
        string executable = Environment.GetEnvironmentVariable(tool.EnvironmentVariable) ?? tool.Name;
        try
        {
            var start = new ProcessStartInfo(executable, tool.VersionArgument) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            // Portable OSS CAD Suite launchers depend on sibling DLLs and
            // helper executables; retain this adjustment within the child only.
            ConfigurePortableSuitePath(start, executable);
            using var process = Process.Start(start);
            if (process is null) return new(tool.Name, tool.EnvironmentVariable, executable, false, null, "process did not start");
            // Begin draining both redirected streams before waiting: an unexpected
            // verbose version banner must not fill a pipe and masquerade as a timeout.
            Task<string> standardOutput = process.StandardOutput.ReadToEndAsync();
            Task<string> standardError = process.StandardError.ReadToEndAsync();
            if (!process.WaitForExit(10000))
            {
                process.Kill(entireProcessTree: true);
                return new(tool.Name, tool.EnvironmentVariable, executable, false, null, "version probe timed out");
            }
            string version = (standardOutput.GetAwaiter().GetResult() + standardError.GetAwaiter().GetResult()).Trim();
            return new(tool.Name, tool.EnvironmentVariable, executable, process.ExitCode == 0, string.IsNullOrWhiteSpace(version) ? null : version, process.ExitCode == 0 ? null : "version probe exited " + process.ExitCode);
        }
        catch (Exception exception) when (exception is System.ComponentModel.Win32Exception or IOException or ArgumentException)
        {
            return new(tool.Name, tool.EnvironmentVariable, executable, false, null, exception.Message);
        }
    }

    internal static void ConfigurePortableSuitePath(ProcessStartInfo start, string executable)
    {
        if (!Path.IsPathRooted(executable)) return;
        string? bin = Path.GetDirectoryName(executable);
        string? root = bin is null ? null : Directory.GetParent(bin)?.FullName;
        string? library = root is null ? null : Path.Combine(root, "lib");
        string prefix = library is not null && Directory.Exists(library) ? bin + Path.PathSeparator + library : bin ?? string.Empty;
        start.Environment["PATH"] = prefix + Path.PathSeparator + Environment.GetEnvironmentVariable("PATH");
    }

    private sealed record Tool(string Name, string EnvironmentVariable, string VersionArgument);
    private sealed record ProbeResult(string name, string environmentVariable, string configured, bool available, string? version, string? reason);
}
