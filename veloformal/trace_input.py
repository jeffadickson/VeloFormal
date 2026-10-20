"""Strict JSON input and a small interpreted expression language.

Expressions use Python boolean/comparison/integer syntax. They are interpreted
from a validated AST; Python eval, attribute access, and indexing are not used.
This is a trace format, not the proposed RTL contract grammar.
"""

import ast
import operator

from .contracts import Contract, Ensure


class InputError(ValueError):
    """An invalid trace document or expression."""


_BINARY = {ast.Add: operator.add, ast.Sub: operator.sub,
           ast.BitAnd: operator.and_, ast.BitOr: operator.or_, ast.BitXor: operator.xor}
_COMPARE = {ast.Eq: operator.eq, ast.NotEq: operator.ne, ast.Lt: operator.lt,
            ast.LtE: operator.le, ast.Gt: operator.gt, ast.GtE: operator.ge}


def expression(source, *, captures=False):
    if not isinstance(source, str) or not source.strip():
        raise InputError("expression must be a nonempty string")
    if len(source) > 4096:
        raise InputError("expression exceeds 4096 characters")
    try:
        root = ast.parse(source, mode="eval").body
    except (SyntaxError, RecursionError) as exc:
        raise InputError(f"invalid expression: {source}") from exc

    def validate(node, in_capture=False):
        if isinstance(node, ast.Constant) and type(node.value) in (bool, int):
            return
        if isinstance(node, ast.Name) and not node.id.startswith("_"):
            return
        if isinstance(node, ast.BoolOp) and isinstance(node.op, (ast.And, ast.Or)):
            for value in node.values:
                validate(value, in_capture)
            return
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.Not, ast.USub, ast.Invert)):
            validate(node.operand, in_capture)
            return
        if isinstance(node, ast.BinOp) and type(node.op) in _BINARY:
            validate(node.left, in_capture)
            validate(node.right, in_capture)
            return
        if isinstance(node, ast.Compare) and all(type(op) in _COMPARE for op in node.ops):
            for child in [node.left, *node.comparators]:
                validate(child, in_capture)
            return
        if (captures and not in_capture and isinstance(node, ast.Call)
                and isinstance(node.func, ast.Name) and node.func.id == "at_trigger"
                and len(node.args) == 1 and not node.keywords):
            validate(node.args[0], True)
            return
        raise InputError(f"unsupported expression syntax: {ast.dump(node)}")

    try:
        validate(root)
    except RecursionError as exc:
        raise InputError("expression nesting is too deep") from exc

    def evaluate(node, sample, captured):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.Name):
            if node.id not in sample:
                raise InputError(f"missing signal: {node.id}")
            return sample[node.id]
        if isinstance(node, ast.BoolOp):
            values = (bool(evaluate(v, sample, captured)) for v in node.values)
            return all(values) if isinstance(node.op, ast.And) else any(values)
        if isinstance(node, ast.UnaryOp):
            value = evaluate(node.operand, sample, captured)
            return (not value if isinstance(node.op, ast.Not) else
                    -value if isinstance(node.op, ast.USub) else ~value)
        if isinstance(node, ast.BinOp):
            return _BINARY[type(node.op)](evaluate(node.left, sample, captured),
                                          evaluate(node.right, sample, captured))
        if isinstance(node, ast.Compare):
            left = evaluate(node.left, sample, captured)
            for op, right_node in zip(node.ops, node.comparators):
                right = evaluate(right_node, sample, captured)
                if not _COMPARE[type(op)](left, right):
                    return False
                left = right
            return True
        return evaluate(node.args[0], captured, None)

    return lambda sample, captured=None: bool(evaluate(root, sample, captured))


def _object(value, allowed, context, required=()):
    if not isinstance(value, dict):
        raise InputError(f"{context} must be an object")
    if set(value) - set(allowed):
        raise InputError(f"unknown {context} fields: {sorted(set(value) - set(allowed))}")
    if set(required) - set(value):
        raise InputError(f"missing {context} fields: {sorted(set(required) - set(value))}")


def load_document(document):
    """Return (Contract, samples) after validating a JSON-decoded document."""
    _object(document, ("contract", "samples"), "document", ("contract", "samples"))
    spec = document["contract"]
    _object(spec, ("requires", "invariants", "ensures", "covers", "reset"), "contract")

    def predicates(role):
        clauses = spec.get(role, {})
        if not isinstance(clauses, dict):
            raise InputError(f"{role} must map labels to expressions")
        return {label: expression(source) for label, source in clauses.items()}

    ensures = spec.get("ensures", [])
    if not isinstance(ensures, list):
        raise InputError("ensures must be an array")
    compiled = []
    for clause in ensures:
        _object(clause, ("label", "when", "response", "after", "within"), "ensure",
                ("label", "when", "response"))
        if ("after" in clause) == ("within" in clause):
            raise InputError("ensure must specify exactly one of after or within")
        bounds = clause.get("within", [clause.get("after"), clause.get("after")])
        if not isinstance(bounds, list) or len(bounds) != 2:
            raise InputError("within must be a two-element array")
        compiled.append(Ensure(clause["label"], expression(clause["when"]),
                               expression(clause["response"], captures=True), *bounds))
    contract = Contract(requires=predicates("requires"), invariants=predicates("invariants"),
                        covers=predicates("covers"), ensures=tuple(compiled),
                        reset=expression(spec.get("reset", "False")))
    samples = document["samples"]
    if not isinstance(samples, list):
        raise InputError("samples must be an array")
    for tick, sample in enumerate(samples):
        if not isinstance(sample, dict) or any(
            not isinstance(key, str) or not key.isidentifier() or key.startswith("_")
            or type(value) not in (bool, int) for key, value in sample.items()
        ):
            raise InputError(f"sample {tick} must map signal names to booleans or integers")
    return contract, samples
