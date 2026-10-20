import unittest

from veloformal import Contract, Ensure, check_trace


class ContractTests(unittest.TestCase):
    def clause(self, earliest=1, latest=1):
        return Ensure("rise", lambda s: s.get("write", False),
                      lambda s, t: s["count"] == t["count"] + 1, earliest, latest)

    def test_invalid_bounds(self):
        for a, b in [(0, 1), (5, 3), (1, 1.5), (True, 1)]:
            with self.subTest(a=a, b=b), self.assertRaises(ValueError):
                self.clause(a, b)

    def test_exact_delay_and_overlap(self):
        report = check_trace(Contract(ensures=(self.clause(),)), [
            {"write": True, "count": 0}, {"write": True, "count": 1}, {"count": 2}])
        self.assertEqual(report.status, "trace-pass")
        self.assertEqual(report.pending, 0)

    def test_late_response_fails_at_deadline(self):
        report = check_trace(Contract(ensures=(self.clause(),)), [
            {"write": True, "count": 0}, {"count": 0}, {"count": 1}])
        self.assertEqual(report.status, "counterexample")
        self.assertEqual((report.findings[0].sample, report.findings[0].trigger_sample), (1, 0))

    def test_overlapping_windows_keep_independent_captures(self):
        report = check_trace(Contract(ensures=(self.clause(1, 2),)), [
            {"write": True, "count": 0}, {"write": True, "count": 0},
            {"count": 0}, {"count": 1}])
        self.assertEqual(report.status, "counterexample")
        self.assertEqual(len(report.findings), 1)
        self.assertEqual(report.findings[0].trigger_sample, 0)
        self.assertEqual(report.pending, 0)

    def test_inclusive_window(self):
        for counts, expected in [([0, 0, 1], "trace-pass"),
                                 ([0, 1, 0], "trace-pass"),
                                 ([0, 0, 0, 1], "counterexample")]:
            with self.subTest(counts=counts):
                report = check_trace(Contract(ensures=(self.clause(1, 2),)),
                                     [{"count": c, "write": i == 0} for i, c in enumerate(counts)])
                self.assertEqual(report.status, expected)

    def test_before_window_does_not_satisfy(self):
        report = check_trace(Contract(ensures=(self.clause(2, 2),)), [
            {"write": True, "count": 0}, {"count": 1}, {"count": 0}])
        self.assertEqual(report.status, "counterexample")

    def test_reset_cancels_and_disables(self):
        contract = Contract(ensures=(self.clause(),), reset=lambda s: s.get("reset", False),
                            invariants={"nonnegative": lambda s: s["count"] >= 0})
        report = check_trace(contract, [{"write": True, "count": 0},
                                       {"reset": True}, {"count": 0}])
        self.assertEqual(report.status, "trace-pass")
        self.assertEqual(report.cancelled, 1)

    def test_pending_is_incomplete(self):
        report = check_trace(Contract(ensures=(self.clause(),)), [{"write": True, "count": 0}])
        self.assertEqual((report.status, report.pending), ("incomplete", 1))

    def test_empty_and_reset_only_are_incomplete(self):
        for samples in ([], [{}]):
            self.assertEqual(check_trace(Contract(reset=lambda s: True), samples).status, "incomplete")

    def test_assumption_failure_is_not_provider_failure(self):
        report = check_trace(Contract(requires={"legal": lambda s: False},
                                     invariants={"safe": lambda s: False}), [{}])
        self.assertEqual(report.status, "invalid-assumptions")
        self.assertEqual([f.label for f in report.findings], ["legal"])

    def test_invariant_and_cover_labels(self):
        report = check_trace(Contract(invariants={"safe": lambda s: s["ok"]},
                                     covers={"bad_seen": lambda s: not s["ok"]}), [{"ok": False}])
        self.assertEqual(report.status, "counterexample")
        self.assertEqual(report.findings[0].label, "safe")
        self.assertEqual(report.covered, ("bad_seen",))

    def test_untriggered_is_reported(self):
        report = check_trace(Contract(ensures=(self.clause(),)), [{"count": 0}])
        self.assertEqual(report.untriggered, ("rise",))

    def test_streamed_mutable_samples_are_captured(self):
        def samples():
            sample = {"count": 0, "write": True}
            yield sample
            sample.update(count=1, write=False)
            yield sample
        self.assertEqual(check_trace(Contract(ensures=(self.clause(),)), samples()).status, "trace-pass")

    def test_duplicate_labels_rejected(self):
        with self.assertRaises(ValueError):
            Contract(invariants={"rise": lambda s: True}, ensures=(self.clause(),))


if __name__ == "__main__":
    unittest.main()
