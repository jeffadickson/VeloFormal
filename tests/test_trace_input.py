import contextlib
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from veloformal import check_trace
from veloformal.__main__ import main
from veloformal.trace_input import InputError, expression, load_document


class TraceInputTests(unittest.TestCase):
    def fixture(self):
        path = Path(__file__).resolve().parents[1] / "examples" / "occupancy.trace"
        return json.loads(path.read_text(encoding="utf-8"))

    def test_example(self):
        report = check_trace(*load_document(self.fixture()))
        self.assertEqual(report.status, "trace-pass")
        self.assertEqual(report.covered, ("two_items",))

    def test_window_and_compound_capture(self):
        document = self.fixture()
        clause = document["contract"]["ensures"][0]
        del clause["after"]
        clause.update(within=[1, 2], response="count == at_trigger(count + 1)")
        self.assertEqual(check_trace(*load_document(document)).status, "trace-pass")

    def test_missing_signal(self):
        document = self.fixture()
        del document["samples"][2]["count"]
        with self.assertRaisesRegex(InputError, "missing signal: count"):
            check_trace(*load_document(document))

    def test_unsupported_expressions(self):
        for source in ["s.value", "s[0]", "__import__('os')", "2 ** 100", "1 / 0",
                       "[1]", "lambda: True", "at_trigger(at_trigger(count))"]:
            with self.subTest(source=source), self.assertRaises(InputError):
                expression(source, captures=True)
        with self.assertRaises(InputError):
            expression("at_trigger(count)")

    def test_short_circuit(self):
        self.assertFalse(expression("False and absent")({}))
        self.assertTrue(expression("True or absent")({}))

    def test_integer_operations(self):
        self.assertTrue(expression("((flags & 3) ^ 1) == 2 and -count < 0")(
            {"flags": 3, "count": 1}))

    def test_invalid_documents(self):
        mutations = [
            lambda d: d.update(unknown=True),
            lambda d: d["contract"].update(clock="clk"),
            lambda d: d["contract"]["ensures"][0].update(within=[1, 2]),
            lambda d: d["contract"]["ensures"][0].update(after=0),
            lambda d: d["contract"]["ensures"][0].update(after=True),
            lambda d: d["samples"][1].update(count=1.5),
            lambda d: d["samples"][1].update(count="X"),
            lambda d: d.update(samples={}),
        ]
        for mutate in mutations:
            document = self.fixture()
            mutate(document)
            with self.subTest(document=document), self.assertRaises(ValueError):
                load_document(document)

    def run_stdin(self, document):
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch("sys.stdin", io.StringIO(document)), contextlib.redirect_stdout(stdout), \
                contextlib.redirect_stderr(stderr):
            code = main(["--trace", "-"])
        return code, stdout.getvalue(), stderr.getvalue()

    def test_cli_pass_and_counterexample(self):
        document = self.fixture()
        code, output, errors = self.run_stdin(json.dumps(document))
        self.assertEqual((code, errors), (0, ""))
        self.assertEqual(json.loads(output)["status"], "trace-pass")
        document["samples"][2]["empty"] = True
        code, output, errors = self.run_stdin(json.dumps(document))
        self.assertEqual((code, errors), (1, ""))
        self.assertEqual(json.loads(output)["findings"][0]["label"], "empty_consistent")

    def test_cli_input_error(self):
        code, output, errors = self.run_stdin("{")
        self.assertEqual((code, output), (2, ""))
        self.assertIn("veloformal:", errors)


if __name__ == "__main__":
    unittest.main()
