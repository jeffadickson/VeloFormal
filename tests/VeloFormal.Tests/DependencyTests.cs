using Xunit;
namespace VeloFormal.Tests;
public sealed class DependencyTests
{
    public static IEnumerable<object[]> Cases()
    {
        string[] sources = ["True", "count", "count + count", "z + a", "a and b", "not ready", "a < b < c",
            "at_trigger(count)", "count == at_trigger(count)", "at_trigger(a + b)", "flags & mask",
            "a or at_trigger(b)", "False and optional", "at_trigger(z) + at_trigger(a)", "((x))", "data_0 == Data",
            "a == 1", "at_trigger(a) == b and at_trigger(a) == c", "10 + 20", "ready or ready"];
        string[] current = ["", "count", "count", "a,z", "a,b", "ready", "a,b,c", "", "count", "", "flags,mask",
            "a", "optional", "", "x", "Data,data_0", "a", "b,c", "", "ready"];
        string[] captured = ["", "", "", "", "", "", "", "count", "count", "a,b", "", "b", "", "a,z", "", "", "", "a", "", ""];
        return sources.Take(20).Select((source, index) => new object[] { 321 + index, source, current[index], captured[index] });
    }
    [Theory, MemberData(nameof(Cases))]
    public void DescribeReferences(int iteration, string source, string current, string captured)
    {
        Assert.InRange(iteration, 321, 340);
        var info = Expression.Describe(source, captures: true);
        Assert.Equal(current, string.Join(',', info.CurrentSignals));
        Assert.Equal(captured, string.Join(',', info.TriggerSignals));
        Assert.Throws<NotSupportedException>(() => ((IList<string>)info.CurrentSignals).Add("mutated"));
        Assert.Throws<NotSupportedException>(() => ((IList<string>)info.TriggerSignals).Add("mutated"));
    }
}
