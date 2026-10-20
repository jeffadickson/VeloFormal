"""Small reproducible demo of the contract trace API."""

import argparse
import json
import sys
from dataclasses import asdict

from .contracts import Contract, Ensure, check_trace
from .trace_input import load_document


def main(argv=None):
    parser = argparse.ArgumentParser(description="VeloFormal experimental contract trace checker")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--demo", action="store_true", help="check a three-sample occupancy trace")
    mode.add_argument("--trace", metavar="FILE", help="check a JSON contract and trace; - reads stdin")
    args = parser.parse_args(argv)
    if args.trace is not None:
        try:
            if args.trace == "-":
                document = json.load(sys.stdin)
            else:
                with open(args.trace, encoding="utf-8") as stream:
                    document = json.load(stream)
            contract, samples = load_document(document)
            report = check_trace(contract, samples)
        except (OSError, ValueError, TypeError, RecursionError) as exc:
            print(f"veloformal: {exc}", file=sys.stderr)
            return 2
        print(json.dumps(asdict(report), indent=2))
        return 0 if report.status == "trace-pass" else 1
    if not args.demo:
        parser.print_help()
        return 0
    contract = Contract(ensures=(Ensure(
        "occupancy_rises", lambda s: s["write"],
        lambda s, trigger: s["count"] == trigger["count"] + 1,
    ),))
    report = check_trace(contract, [
        {"write": True, "count": 0},
        {"write": True, "count": 1},
        {"write": False, "count": 2},
    ])
    print(json.dumps(asdict(report), indent=2))
    return 0 if report.status == "trace-pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
