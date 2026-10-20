"""Check sampled traces against a small executable subset of contract v0.2.

Predicates are Python callables over sample mappings. Ensure predicates also
receive the captured trigger sample. This API does not parse or type-check RTL.
Every supplied sample represents one active clock edge, including reset edges.
"""

from copy import deepcopy
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable, Mapping

Sample = Mapping[str, Any]
Predicate = Callable[[Sample], bool]
Response = Callable[[Sample, Sample], bool]


@dataclass(frozen=True)
class Ensure:
    label: str
    trigger: Predicate
    response: Response
    earliest: int = 1
    latest: int = 1

    def __post_init__(self):
        if (type(self.earliest) is not int or type(self.latest) is not int
                or not 1 <= self.earliest <= self.latest):
            raise ValueError("delay bounds must be integers with 1 <= earliest <= latest")


@dataclass(frozen=True)
class Contract:
    requires: Mapping[str, Predicate] = field(default_factory=dict)
    invariants: Mapping[str, Predicate] = field(default_factory=dict)
    ensures: tuple[Ensure, ...] = ()
    covers: Mapping[str, Predicate] = field(default_factory=dict)
    reset: Predicate = lambda sample: False

    def __post_init__(self):
        labels = [*self.requires, *self.invariants,
                  *(clause.label for clause in self.ensures), *self.covers]
        if any(not isinstance(label, str) or not label.strip() for label in labels):
            raise ValueError("clause labels must be nonempty strings")
        if len(labels) != len(set(labels)):
            raise ValueError("clause labels must be unique")


@dataclass(frozen=True)
class Finding:
    label: str
    sample: int
    kind: str
    trigger_sample: int | None = None


@dataclass(frozen=True)
class Report:
    status: str
    findings: tuple[Finding, ...]
    pending: int
    cancelled: int
    active_samples: int
    untriggered: tuple[str, ...]
    covered: tuple[str, ...]


@dataclass
class _Obligation:
    clause: Ensure
    start: int
    capture: Sample


def check_trace(contract: Contract, samples: Iterable[Sample]) -> Report:
    """Observe a finite trace; successful observation never constitutes proof.

    A failed require invalidates the whole trace as provider evidence. Findings
    from that sample onward are suppressed. Untriggered means unobserved in this
    trace, not formally unreachable. Outstanding deadlines yield incomplete.
    """
    pending: list[_Obligation] = []
    findings: list[Finding] = []
    triggered: set[str] = set()
    covered: set[str] = set()
    cancelled = active = 0
    invalid = False
    for tick, raw_sample in enumerate(samples):
        sample = deepcopy(dict(raw_sample))
        if contract.reset(sample):
            cancelled += len(pending)
            pending.clear()
            continue
        active += 1
        failures = [label for label, pred in contract.requires.items() if not pred(sample)]
        if failures:
            findings.extend(Finding(label, tick, "require") for label in failures)
            invalid = True
            break
        for label, pred in contract.invariants.items():
            if not pred(sample):
                findings.append(Finding(label, tick, "invariant"))
        remaining = []
        for obligation in pending:
            clause = obligation.clause
            age = tick - obligation.start
            if age >= clause.earliest and clause.response(sample, obligation.capture):
                continue
            if age == clause.latest:
                findings.append(Finding(clause.label, tick, "ensure", obligation.start))
            else:
                remaining.append(obligation)
        pending = remaining
        for clause in contract.ensures:
            if clause.trigger(sample):
                triggered.add(clause.label)
                pending.append(_Obligation(clause, tick, deepcopy(sample)))
        covered.update(label for label, pred in contract.covers.items() if pred(sample))
    status = ("invalid-assumptions" if invalid else "counterexample" if findings
              else "incomplete" if pending or not active else "trace-pass")
    return Report(status, tuple(findings), len(pending), cancelled, active,
                  tuple(c.label for c in contract.ensures if c.label not in triggered),
                  tuple(label for label in contract.covers if label in covered))
