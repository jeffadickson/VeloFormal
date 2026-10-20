using System.Globalization;
using System.Numerics;

namespace VeloFormal;

/// <summary>Unsigned two-state samples at settled rising edges of a scalar VCD clock.</summary>
public static class VcdReader
{
    private sealed record Variable(string Id, int Width);
    public static IReadOnlyList<Sample> Read(string text, string clock,
        IReadOnlyDictionary<string, string> signals)
    {
        if (signals.Count == 0) throw new InputException("select at least one VCD signal");
        var tokens = text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
        int index = 0;
        string Next() => index < tokens.Length ? tokens[index++] : throw new InputException("truncated VCD");
        void End() { while (Next() != "$end") { } }
        var scope = new List<string>();
        var variables = new Dictionary<string, Variable>(StringComparer.Ordinal);
        while (true)
        {
            string token = Next();
            if (token == "$enddefinitions") { if (Next() != "$end") throw new InputException("invalid VCD header"); break; }
            if (token == "$scope") { Next(); scope.Add(Next()); if (Next() != "$end") throw new InputException("invalid scope"); }
            else if (token == "$upscope")
            {
                if (scope.Count == 0) throw new InputException("unbalanced VCD scope");
                scope.RemoveAt(scope.Count - 1);
                if (Next() != "$end") throw new InputException("invalid upscope");
            }
            else if (token == "$var")
            {
                string type = Next();
                if (!int.TryParse(Next(), out int width) || width is < 1 or > 65536)
                    throw new InputException("invalid VCD width");
                string id = Next(), name = Next();
                if (type is not "wire" and not "reg" and not "integer" and not "logic" and not "parameter")
                    throw new InputException($"unsupported VCD type: {type}");
                string path = string.Join('.', scope.Append(name));
                if (!variables.TryAdd(path, new(id, width))) throw new InputException($"duplicate VCD signal: {path}");
                End();
            }
            else if (token is "$date" or "$version" or "$timescale" or "$comment") End();
            else throw new InputException($"unsupported VCD header: {token}");
        }
        Variable Resolve(string path) => variables.TryGetValue(path, out var value) ? value
            : throw new InputException($"VCD signal not found: {path}");
        var clockVariable = Resolve(clock);
        if (clockVariable.Width != 1) throw new InputException("VCD clock must be scalar");
        var selected = signals.ToDictionary(p => p.Key, p => Resolve(p.Value), StringComparer.Ordinal);
        var byId = variables.Values.GroupBy(v => v.Id).ToDictionary(g => g.Key, g => g.First());
        if (variables.Values.GroupBy(v => v.Id).Any(g => g.Select(v => v.Width).Distinct().Count() > 1))
            throw new InputException("VCD aliases have inconsistent widths");
        var values = new Dictionary<string, string>(StringComparer.Ordinal);
        var samples = new List<Sample>();
        string? oldClock = null;
        long time = -1;
        int clockChanges = 0;
        void Flush()
        {
            values.TryGetValue(clockVariable.Id, out var current);
            if (clockChanges > 1) throw new InputException($"multiple VCD clock changes at time {time}");
            if (oldClock == "0" && current == "1")
            {
                var sample = new Dictionary<string, BigInteger>(StringComparer.Ordinal);
                foreach (var (name, variable) in selected)
                {
                    if (!values.TryGetValue(variable.Id, out var bits) || bits.Any(c => c is 'x' or 'z'))
                        throw new InputException($"unknown VCD value for {name} at time {time}");
                    BigInteger value = 0;
                    foreach (char bit in bits) value = (value << 1) + (bit - '0');
                    sample.Add(name, value);
                }
                samples.Add(new(sample));
            }
            oldClock = current;
            clockChanges = 0;
        }
        while (index < tokens.Length)
        {
            var token = Next();
            if (token.StartsWith('#'))
            {
                if (!long.TryParse(token.AsSpan(1), NumberStyles.None, CultureInfo.InvariantCulture, out long next) || next < time)
                    throw new InputException("invalid or decreasing VCD time");
                if (next != time) { Flush(); time = next; }
            }
            else if (token is "$dumpvars" or "$dumpall" or "$end") { }
            else if (token == "$comment") End();
            else
            {
                string id, bits;
                if (token[0] is 'b' or 'B') { bits = token[1..].ToLowerInvariant(); id = Next(); }
                else if (token[0] is '0' or '1' or 'x' or 'X' or 'z' or 'Z')
                { bits = token[..1].ToLowerInvariant(); id = token[1..]; }
                else throw new InputException($"unsupported VCD value: {token}");
                if (!byId.TryGetValue(id, out var variable)) throw new InputException($"undeclared VCD identifier: {id}");
                if (bits.Length == 0 || bits.Length > variable.Width || bits.Any(c => c is not '0' and not '1' and not 'x' and not 'z'))
                    throw new InputException("invalid VCD bit vector");
                if (id == clockVariable.Id && (!values.TryGetValue(id, out var prior) || prior != bits)) clockChanges++;
                values[id] = bits;
            }
        }
        Flush();
        return samples.AsReadOnly();
    }
}
