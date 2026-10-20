namespace VeloFormal.Build;
public sealed record BuildOptions(string Mode, bool NoBuild = false, bool NoRestore = false)
{
    public const string Usage = "usage: dotnet run --project tools/VeloFormal.Build -- [test|coverage|rtl] [--no-build] [--no-restore]";
    public static BuildOptions Parse(string[] args)
    {
        ArgumentNullException.ThrowIfNull(args);
        if (args is ["--help"] or ["-h"]) return new("help");
        string? mode = null;
        var flags = new HashSet<string>(StringComparer.Ordinal);
        foreach (string argument in args)
        {
            if (argument is "test" or "coverage" or "rtl")
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
        if (mode == "rtl" && flags.Count != 0) throw new InputException("build flags do not apply to rtl mode");
        return new(mode, flags.Contains("--no-build"), flags.Contains("--no-restore"));
    }
}
