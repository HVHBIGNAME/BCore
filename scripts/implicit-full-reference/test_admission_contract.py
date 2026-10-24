"""Contract regressions: native concurrency is not a timestamp or return-order rule."""

import unittest

from admission_contract import check_pumps, match_runnables, pair_calls


def obj(ident):
    return {"object_id": ident, "class": "java.util.concurrent.CompletableFuture"}


class ContractTests(unittest.TestCase):
    def test_execution_can_precede_schedule_return(self):
        schedule = {"family": "main_schedule", "enter": {"call_id": 1, "seq": 1, "request": 0,
                    "executor": obj(10), "runnable": obj(11)}, "return": {"seq": 4}}
        execute = {"family": "main_execute", "enter": {"call_id": 2, "seq": 2, "request": 1,
                   "executor": obj(10), "runnable": obj(11)}, "return": {"seq": 3}}
        links, pending = match_runnables({1: schedule, 2: execute})
        self.assertTrue(links[0]["execution_before_schedule_return"])
        self.assertFalse(pending)

    def test_wrong_runnable_identity_is_rejected(self):
        execute = {"family": "main_execute", "enter": {"call_id": 1, "seq": 1, "request": 0,
                   "executor": obj(10), "runnable": obj(11)}, "return": {"seq": 2}}
        with self.assertRaisesRegex(ValueError, "unmatched/ambiguous"):
            match_runnables({1: execute})

    def test_pending_submission_is_retained(self):
        schedule = {"family": "main_schedule", "enter": {"call_id": 1, "seq": 1, "request": 0,
                    "executor": obj(10), "runnable": obj(11)}, "return": {"seq": 2}}
        links, pending = match_runnables({1: schedule})
        self.assertFalse(links)
        self.assertEqual(len(pending), 1)

    def test_future_cannot_become_incomplete(self):
        events = [{"schema": 1, "seq": 1, "baseline_seq": 1, "event": "admission_start",
                   "future": {**obj(2), "done": True}},
                  {"schema": 1, "seq": 2, "baseline_seq": 1, "event": "full_supplier",
                   "future": {**obj(2), "done": False}}]
        with self.assertRaisesRegex(ValueError, "became incomplete"):
            pair_calls(events, 2)

    @staticmethod
    def pump_calls(ready, polls):
        pending = {"runnables": [obj(99)], "truncated": False}
        return {1: {"family": "api_future", "return": {"seq": 1, "api_call_id": 4, "future": obj(12)}},
                2: {"family": "pump", "enter": {"seq": 2, "call_id": 2, "api_call_id": 4,
                    "request": 0, "future_known": True, "requested_future": {**obj(12), "done": ready}, "pending": pending},
                    "return": {"seq": 3, "requested_future": {**obj(12), "done": True}, "pending": pending,
                               "counters": {"main_runs": 0, "polls": polls, "worked_polls": 0}}}}

    def test_already_ready_pump_must_not_poll(self):
        with self.assertRaisesRegex(ValueError, "already-ready"):
            check_pumps(self.pump_calls(True, 1))

    def test_readiness_can_change_before_native_guard(self):
        # Instrumented entry precedes the native guard: false then zero polls is valid.
        report, = check_pumps(self.pump_calls(False, 0))
        self.assertEqual(report["pending_ids_observed_at_both_boundaries"], [99])


if __name__ == "__main__":
    unittest.main()
