namespace VeloFormal.Build;
public sealed record BuildOptions(string Mode, bool NoBuild = false, bool NoRestore = false)
{
    public const string Usage = "usage: dotnet run --project tools/VeloFormal.Build -- [test|coverage|rtl|yosys-probe|yosys-stage1|yosys-stage1-positive|yosys-stage1-deterministic] [--no-build] [--no-restore]";
    public static BuildOptions Parse(string[] args)
    {
        ArgumentNullException.ThrowIfNull(args);
        if (args is ["--help"] or ["-h"]) return new("help");
        string? mode = null;
        var flags = new HashSet<string>(StringComparer.Ordinal);
        foreach (string argument in args)
        {
            if (argument is "test" or "coverage" or "rtl" or "yosys-probe" or "yosys-stage1" or "yosys-stage1-positive" or "yosys-stage1-deterministic")
            {
                if (mode is not null) throw new InputException("specify one build mode");
                mode = argument;
            }
            else if (argument is "--no-build" or "--no-restore")
            {
                if (!flags.Add(argument)) throw new InputException($"duplicate build option: {argument}");
            }
            else throw new InputException($"unknown build argument: {argument}");
        }
        mode ??= "test";
        if (mode is "rtl" or "yosys-probe" or "yosys-stage1" or "yosys-stage1-positive" or "yosys-stage1-deterministic" && flags.Count != 0) throw new InputException("build flags do not apply to this mode");
        return new(mode, flags.Contains("--no-build"), flags.Contains("--no-restore"));
    }
}
