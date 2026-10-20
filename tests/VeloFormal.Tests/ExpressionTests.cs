using System.Numerics;
using Xunit;

namespace VeloFormal.Tests;

public sealed class ExpressionTests
{
    public static IEnumerable<object[]> Cases()
    {
        // Iterations 001..050: expression compatibility and operator boundaries.
        string[] expressions = [
            "True", "False", "0", "1", "-1", "not 0", "not 1", "not not 1",
            "1 + 2 == 3", "4 - 3 == 1", "1 - 3 == -2", "-(-1) == 1",
            "~0 == -1", "~1 == -2", "3 & 1 == 1", "2 | 1 == 3", "3 ^ 1 == 2",
            "1 < 2", "2 <= 2", "2 > 1", "2 >= 2", "2 == 2", "2 != 3",
            "1 < 2 < 3", "1 < 2 > 1", "3 < 2 < absent", "False and absent", "True or absent",
            "True and False", "False or True", "not 1 == 2", "1 == 1 and 2 == 2",
            "1 == 2 or 2 == 2", "1 + 2 & 1 == 1", "1 | 2 & 0 == 1",
            "1 ^ 3 & 1 == 0", "1 - 2 - 3 == -4", "1 + (2 - 3) == 0",
            "not (True and False)", "count == 4", "count + 1 == 5", "count - 5 == -1",
            "0 <= count <= 4", "flags & 3 == 3", "(flags & 3) ^ 1 == 2",
            "999999999999999999999999999999999999999999 + 1 == 1000000000000000000000000000000000000000000",
            "-999999999999999999999 < -1", "True == 1", "False == 0", " count\n ==\t4 "
        ];
        int[] falseCases = [2, 3, 7, 26, 27, 29];
        return expressions.Select((expression, index) => new object[] { index + 1, expression, !falseCases.Contains(index + 1) });
    }

    [Theory]
    [MemberData(nameof(Cases))]
    public void Evaluate(int iteration, string source, bool expected)
    {
        Assert.InRange(iteration, 1, 50);
        var sample = new Sample(new Dictionary<string, BigInteger> { ["count"] = 4, ["flags"] = 3 });
        Assert.Equal(expected, Expression.Predicate(source)(sample));
    }
}
