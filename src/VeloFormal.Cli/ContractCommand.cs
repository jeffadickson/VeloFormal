using System.Globalization;
using VeloFormal.Language;
using VeloFormal.Solving;

namespace VeloFormal;

/// <summary>
/// Adapts the v0.2 contract parser and finite bounded checker to the command line.
/// Models are explicit transition relations; this command intentionally does not claim
/// to elaborate or formally prove arbitrary Verilog source.
/// </summary>
internal static class ContractCommand
{
    internal static object Parse(string[] args)
    {
        string path = RequiredValue(args, "--parse-contract");
        RejectUnknown(args, "--parse-contract");
        var contract = ContractParser.Parse(Read(path), path);
        return new
        {
            status = "parsed",
            contract = contract.Name,
            module = contract.Module,
            clock = contract.Clock,
            reset = contract.Reset is not null,
            parameters = contract.Parameters.Select(parameter => parameter.Name).ToArray(),
            clauses = contract.Clauses.Select(clause => new { clause.Role, clause.Label }).ToArray(),
            operations = contract.Operations.Select(operation => new
            {
                operation.Name,
                ensures = operation.Ensures.Select(ensure => new { ensure.Label, earliest_expression = Slice(contract.Source, ensure.Earliest.Span), latest_expression = Slice(contract.Source, ensure.Latest.Span) }).ToArray()
            }).ToArray()
        };
    }

    internal static SolverReport Check(string[] args)
    {
        string contractPath = RequiredValue(args, "--check-contract");
        string modelPath = RequiredValue(args, "--model");
        int depth = OptionalPositiveInt(args, "--depth", 16, 0, 128);
        uint timeout = (uint)OptionalPositiveInt(args, "--timeout-ms", 10000, 1, 60000);
        int maxQueries = OptionalPositiveInt(args, "--max-queries", 4096, 1, 10000);
        RejectUnknown(args, "--check-contract", "--model", "--depth", "--timeout-ms", "--max-queries");
        var contract = ContractParser.Parse(Read(contractPath), contractPath);
        var model = TransitionModel.Parse(Read(modelPath), modelPath);
        return BoundedChecker.Check(SemanticBinder.Bind(contract, model), new SolverOptions(Depth: depth, TimeoutMilliseconds: timeout, MaxQueries: maxQueries));
    }

    /// <summary>Returns the original expression spelling for a source-located syntax node.</summary>
    private static string Slice(string source, SourceSpan span) => source.Substring(span.Start, span.Length);

    private static string Read(string path)
    {
        if (path == "-") throw new InputException("contract and model commands require named files");
        return File.ReadAllText(path);
    }

    private static string RequiredValue(string[] args, string option)
    {
        int index = Array.IndexOf(args, option);
        if (index < 0 || index + 1 >= args.Length || args[index + 1].StartsWith("--", StringComparison.Ordinal))
            throw new InputException($"expected {option} FILE");
        if (Array.IndexOf(args, option, index + 1) >= 0) throw new InputException($"duplicate {option}");
        return args[index + 1];
    }

    private static int OptionalPositiveInt(string[] args, string option, int fallback, int minimum, int maximum)
    {
        int index = Array.IndexOf(args, option);
        if (index < 0) return fallback;
        if (index + 1 >= args.Length || !int.TryParse(args[index + 1], NumberStyles.None, CultureInfo.InvariantCulture, out int value) || value < minimum || value > maximum)
            throw new InputException($"{option} must be an integer from {minimum} through {maximum}");
        if (Array.IndexOf(args, option, index + 1) >= 0) throw new InputException($"duplicate {option}");
        return value;
    }

    private static void RejectUnknown(string[] args, params string[] options)
    {
        var known = new HashSet<string>(options, StringComparer.Ordinal);
        for (int index = 0; index < args.Length; index++)
        {
            if (!known.Contains(args[index])) throw new InputException($"unsupported option {args[index]}");
            index++;
            if (index >= args.Length) throw new InputException($"missing value for {args[index - 1]}");
        }
    }
}
