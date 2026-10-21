using System.Globalization;
using Microsoft.Z3;
using VeloFormal.Language;

namespace VeloFormal.Solving;
internal sealed class Z3Translation(Context context, TransitionModel model, int maximum)
{
    private readonly Dictionary<(TypedExpression, int, int?), Expr> cache = new();
    private readonly Dictionary<(string, int), Expr> variables = new();
    private readonly Dictionary<string, int> indices = model.Signals.Select((s, i) => (s.Name, i)).ToDictionary(p => p.Name, p => p.i, StringComparer.Ordinal);
    public Expr Variable(SignalDeclaration signal, int frame)
    {
        if (variables.TryGetValue((signal.Name, frame), out var value)) return value;
        Sort type = signal.Type.Kind == SortKind.Boolean ? context.BoolSort : context.MkBitVecSort((uint)signal.Type.Width);
        value = context.MkConst($"s{indices[signal.Name]}_t{frame}", type);
        variables.Add((signal.Name, frame), value); return value;
    }
    public BoolExpr Boolean(TypedExpression expression, int frame, int? trigger = null) => (BoolExpr)Translate(expression, frame, trigger);
    public Expr Translate(TypedExpression node, int frame, int? trigger = null)
    {
        if (cache.TryGetValue((node, frame, trigger), out var found)) return found;
        if (cache.Count >= maximum) throw new SolverBudgetException("expression budget exceeded");
        Expr Child(int index) => Translate(node.Children[index], frame, trigger);
        Expr result;
        switch (node.Kind)
        {
            case "constant":
                result = node.Type.Kind == SortKind.Boolean ? context.MkBool(!node.Value.IsZero)
                    : node.Type.Kind == SortKind.Integer ? context.MkInt(node.Value.ToString(CultureInfo.InvariantCulture))
                    : context.MkBV(node.Value.ToString(CultureInfo.InvariantCulture), (uint)node.Type.Width); break;
            case "name": result = Variable(model.Signals[indices[node.Text]], frame); break;
            case "capture": result = Translate(node.Children[0], trigger ?? throw new InputException("capture frame missing"), null); break;
            case "group": result = Child(0); break;
            case "cast":
                var source = (BitVecExpr)Child(0);
                uint extension = (uint)(node.Type.Width - node.Children[0].Type.Width);
                result = extension == 0 ? source : node.Children[0].Type.Signed ? context.MkSignExt(extension, source) : context.MkZeroExt(extension, source); break;
            case "ite": result = context.MkITE((BoolExpr)Child(0), Child(1), Child(2)); break;
            case "unary":
                result = node.Text switch { "!" => context.MkNot((BoolExpr)Child(0)), "~" => context.MkBVNot((BitVecExpr)Child(0)),
                    "-" => context.MkBVNeg((BitVecExpr)Child(0)), "+" => Child(0), _ => throw new InputException("unsupported typed unary operator") }; break;
            case "binary":
                var left = Child(0); var right = Child(1);
                bool signed = node.Children[0].Type.Signed;
                result = node.Text switch
                {
                    "&&" => context.MkAnd((BoolExpr)left, (BoolExpr)right), "||" => context.MkOr((BoolExpr)left, (BoolExpr)right),
                    "==" => context.MkEq(left, right), "!=" => context.MkNot(context.MkEq(left, right)),
                    "+" => context.MkBVAdd((BitVecExpr)left, (BitVecExpr)right), "-" => context.MkBVSub((BitVecExpr)left, (BitVecExpr)right),
                    "*" => context.MkBVMul((BitVecExpr)left, (BitVecExpr)right), "&" => context.MkBVAND((BitVecExpr)left, (BitVecExpr)right),
                    "|" => context.MkBVOR((BitVecExpr)left, (BitVecExpr)right), "^" => context.MkBVXOR((BitVecExpr)left, (BitVecExpr)right),
                    "<<" => context.MkBVSHL((BitVecExpr)left, (BitVecExpr)right),
                    ">>" => signed ? context.MkBVASHR((BitVecExpr)left, (BitVecExpr)right) : context.MkBVLSHR((BitVecExpr)left, (BitVecExpr)right),
                    "<" => signed ? context.MkBVSLT((BitVecExpr)left, (BitVecExpr)right) : context.MkBVULT((BitVecExpr)left, (BitVecExpr)right),
                    "<=" => signed ? context.MkBVSLE((BitVecExpr)left, (BitVecExpr)right) : context.MkBVULE((BitVecExpr)left, (BitVecExpr)right),
                    ">" => signed ? context.MkBVSGT((BitVecExpr)left, (BitVecExpr)right) : context.MkBVUGT((BitVecExpr)left, (BitVecExpr)right),
                    ">=" => signed ? context.MkBVSGE((BitVecExpr)left, (BitVecExpr)right) : context.MkBVUGE((BitVecExpr)left, (BitVecExpr)right),
                    _ => throw new InputException("unsupported typed binary operator")
                }; break;
            default: throw new InputException("unsupported typed node");
        }
        if (cache.Count >= maximum) throw new SolverBudgetException("expression budget exceeded");
        cache.Add((node, frame, trigger), result); return result;
    }
}
internal sealed class SolverBudgetException(string message) : Exception(message);
