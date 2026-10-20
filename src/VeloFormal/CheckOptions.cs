namespace VeloFormal;
public sealed record CheckOptions(int MaxSamples = 1000000, int MaxPending = 100000, int MaxFindings = 100000)
{
    internal void Validate()
    {
        if (MaxSamples < 1 || MaxPending < 1 || MaxFindings < 1)
            throw new InputException("trace limits must be positive");
    }
}
