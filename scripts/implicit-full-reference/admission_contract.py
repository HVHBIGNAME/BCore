"""Identity and partial-order checks for private-companion-v1 sidecars."""

from collections import Counter, defaultdict


FAMILIES = {
    "api", "api_future", "cache_store", "cache_clear", "pump", "main_schedule",
    "main_execute", "holder_update", "holder_future", "accessible", "full",
    "full_body", "full_status_supplier",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def object_id(value):
    return value.get("object_id") if value else None


def object_values(value):
    if isinstance(value, dict):
        if "object_id" in value:
            yield value
        for item in value.values():
            yield from object_values(item)
    elif isinstance(value, list):
        for item in value:
            yield from object_values(item)


def pair_calls(events, terminal_seq):
    calls, stacks, types, done = {}, defaultdict(list), {}, set()
    previous_baseline = 0
    for seq, event in enumerate(events, 1):
        require(event["schema"] == 1 and event["seq"] == seq, "invalid sidecar schema/sequence")
        baseline = event["baseline_seq"]
        require(previous_baseline <= baseline <= terminal_seq, "invalid sampled baseline order")
        previous_baseline = baseline
        for value in object_values(event):
            ident = object_id(value)
            require(isinstance(ident, int) and ident > 0, "invalid companion object ID")
            require(types.setdefault(ident, value["class"]) == value["class"], "object ID changed class")
            if "done" in value:
                require(value["done"] or ident not in done, "completed future became incomplete")
                if value["done"]:
                    done.add(ident)
        stem, _, boundary = event["event"].rpartition("_")
        if stem not in FAMILIES or boundary not in ("enter", "return"):
            continue
        ident, thread = event["call_id"], event["thread_id"]
        if boundary == "enter":
            require(ident not in calls, "duplicate invocation entry")
            calls[ident] = {"family": stem, "enter": event}
            stacks[thread].append(ident)
        else:
            require(ident in calls and calls[ident]["family"] == stem, "return lacks matching entry")
            require(stacks[thread] and stacks[thread][-1] == ident, "non-nested same-thread return")
            require("return" not in calls[ident], "duplicate invocation return")
            require(calls[ident]["enter"]["method"] == event["method"], "invocation changed method")
            stacks[thread].pop()
            calls[ident]["return"] = event
    require(events and events[0]["event"] == "admission_start", "missing sidecar start")
    return calls, types


def family(calls, name):
    return [call for call in calls.values() if call["family"] == name]


def match_runnables(calls):
    schedules = defaultdict(list)
    for call in family(calls, "main_schedule"):
        event = call["enter"]
        schedules[object_id(event["executor"]), object_id(event["runnable"])].append(call)
    used, links = set(), []
    for execution in family(calls, "main_execute"):
        event = execution["enter"]
        key = object_id(event["executor"]), object_id(event["runnable"])
        candidates = [call for call in schedules[key] if call["enter"]["call_id"] not in used
                      and call["enter"]["seq"] < event["seq"]]
        require(len(candidates) == 1, f"unmatched/ambiguous runnable execution at sidecar {event['seq']}")
        require("return" in execution, "main execution is still open at baseline halt")
        schedule = candidates[0]
        used.add(schedule["enter"]["call_id"])
        returned = schedule.get("return")
        # A different thread may dequeue/run after Queue.add and before schedule returns.
        links.append({
            "runnable_id": key[1], "executor_id": key[0],
            "schedule_call_id": schedule["enter"]["call_id"], "execution_call_id": event["call_id"],
            "schedule_enter_seq": schedule["enter"]["seq"],
            "schedule_return_seq": returned["seq"] if returned else None,
            "execute_enter_seq": event["seq"], "execute_return_seq": execution["return"]["seq"],
            "execution_before_schedule_return": returned is None or event["seq"] < returned["seq"],
            "full_call_id": schedule["enter"].get("full_call_id"),
            "pump_call_id": event.get("pump_call_id"),
            "api_call_id": event.get("api_call_id"),
            "submission_request_label": schedule["enter"]["request"],
            "execution_request_label": event["request"],
        })
    pending = [call for call in family(calls, "main_schedule") if call["enter"]["call_id"] not in used]
    return links, pending


def cache_signature(slots):
    return [(slot["slot"], slot["packed_pos"], slot.get("requested_status"), object_id(slot.get("chunk"))) for slot in slots]


def check_caches(calls):
    for call in family(calls, "cache_store"):
        require("return" in call, "cache store lacks a return")
        before, after = call["enter"], call["return"]
        left, right = cache_signature(before["cache_slots"]), cache_signature(after["cache_slots"])
        require(len(left) == len(right) == 4, "unexpected cache size")
        require([row[1:] for row in left[:3]] == [row[1:] for row in right[1:]], "cache store did not shift slots")
        require(right[0][1:] == (before["packed_pos"], before["stored_status"], object_id(before.get("stored_chunk"))),
                "cache store slot zero differs from its arguments")
    for call in family(calls, "cache_clear"):
        require("return" in call, "cache clear lacks a return")
        slots = call["return"]["cache_slots"]
        require(len(slots) == 4 and all(not slot.get("chunk") and not slot.get("requested_status") for slot in slots),
                "cache clear retained a status/object entry")


def check_pumps(calls):
    futures = family(calls, "api_future")
    executions = family(calls, "main_execute")
    reports = []
    for call in family(calls, "pump"):
        require("return" in call, "pump lacks a return")
        before, after = call["enter"], call["return"]
        require(before["future_known"], "pump requested-future identity is unavailable")
        ident = object_id(before["requested_future"])
        require(ident == object_id(after["requested_future"]) and after["requested_future"]["done"],
                "pump returned with wrong/incomplete future")
        origins = [other["return"] for other in futures if "return" in other
                   and other["return"].get("api_call_id") == before.get("api_call_id")
                   and other["return"]["seq"] < before["seq"]
                   and object_id(other["return"]["future"]) == ident]
        require(origins, "pump future does not match its preceding API future return")
        counters = after["counters"]
        executed = [other["enter"]["call_id"] for other in executions
                    if other["enter"].get("pump_call_id") == before["call_id"]]
        require(counters["main_runs"] == len(executed), "pump main-run counter differs from execution events")
        require(0 <= counters["worked_polls"] <= counters["polls"], "invalid poll counters")
        if before["requested_future"]["done"]:
            require(counters["polls"] == 0 and not executed, "already-ready pump performed work")
        before_pending = {object_id(row) for row in before["pending"]["runnables"]}
        after_pending = {object_id(row) for row in after["pending"]["runnables"]}
        reports.append({"pump_call_id": before["call_id"], "api_call_id": before.get("api_call_id"),
            "request_label": before["request"], "enter_seq": before["seq"], "return_seq": after["seq"],
            "future_id": ident, "ready_at_entry": before["requested_future"]["done"], "counters": counters,
            "pending_before": before["pending"], "pending_after": after["pending"],
            "pending_ids_observed_at_both_boundaries": sorted(before_pending & after_pending),
            "executed_call_ids": executed})
    return reports


def check_apis(calls, specs):
    reports = []
    for call in family(calls, "api"):
        before = call["enter"]
        if before.get("parent_api_call_id") or before.get("baseline_observing"):
            continue
        require("return" in call, "primary API call lacks return")
        after = call["return"]
        futures = [other for other in family(calls, "api_future") if other["enter"].get("api_call_id") == before["call_id"]]
        pumps = [other for other in family(calls, "pump") if other["enter"].get("api_call_id") == before["call_id"]]
        require(after["counters"]["future_calls"] == len(futures), "API future-call counter mismatch")
        require(after["counters"]["pump_calls"] == len(pumps), "API pump counter mismatch")
        if not futures:
            require(not pumps, "cache-only API call pumped")
            slots = [slot for slot in before["cache_slots"] if slot.get("requested_status") == before["requested_status"]
                     and slot["pos"] == before["pos"] and (slot.get("chunk") or not before["create"])]
            require(slots and object_id(slots[0].get("chunk")) == object_id(after.get("returned")), "cache hit returned different object")
            require(cache_signature(before["cache_slots"]) == cache_signature(after["cache_slots"]), "cache hit changed slot order")
        reports.append({"request_label": before["request"], "api_call_id": before["call_id"],
            "enter_seq": before["seq"], "return_seq": after["seq"], "pos": before["pos"],
            "requested_status": before["requested_status"], "returned": after.get("returned"),
            "path": "future" if futures else "cache", "counters": after["counters"],
            "cache_before": before["cache_slots"], "cache_after": after["cache_slots"]})
    require(len(reports) == len(specs), "primary API count differs from scripted requests")
    for report, spec in zip(reports, specs, strict=True):
        require(report["pos"] == spec["pos"] and report["requested_status"] == "minecraft:" + spec["status"],
                "primary API does not match scripted request")
    return reports


def check_accessibility(calls):
    reports = []
    for call in family(calls, "accessible"):
        require("return" in call, "accessible preparation lacks a return")
        before, after = call["enter"], call["return"]
        update = calls[before["holder_update_call_id"]]
        require(update["family"] == "holder_update" and "return" in update, "accessibility lacks enclosing update")
        holder = update["enter"]["holder"]
        require(holder["old_ticket_level"] > 33 >= holder["ticket_level"], "accessibility is not a FULL threshold transition")
        require(object_id(holder) == object_id(before["holder"]), "accessibility changed holder identity")
        require(object_id(after["future"]) == object_id(update["return"]["holder"]["full_chunk_future"]),
                "accessible future differs from installed holder future")
        reports.append({"pos": holder["pos"], "holder_id": object_id(holder),
            "old_ticket_level": holder["old_ticket_level"], "ticket_level": holder["ticket_level"],
            "update_enter_seq": update["enter"]["seq"], "update_return_seq": update["return"]["seq"],
            "accessible_enter_seq": before["seq"], "accessible_return_seq": after["seq"],
            "full_chunk_future_id": object_id(after["future"]), "request_label": before["request"]})
    return reports


def full_chains(events, calls, links, stages, stage_exits):
    reports = []
    for call in family(calls, "full"):
        before = call["enter"]
        stage_id = before["baseline_stage_id"]
        require(stage_id in stages and stages[stage_id]["status"] == "minecraft:full", "full invocation lacks baseline stage ID")
        require(stages[stage_id]["pos"] == before["input_chunk"]["pos"], "full stage/input position mismatch")
        suppliers = [row for row in events if row["event"] == "full_supplier" and row["call_id"] == before["call_id"]]
        schedules = [other for other in family(calls, "main_schedule") if other["enter"].get("full_call_id") == before["call_id"]]
        if "return" in call:
            require(len(suppliers) == len(schedules) == 1, "completed full submission lacks unique supplier/runnable")
            require(object_id(suppliers[0]["supplier"]) == object_id(call["return"]["supplier"]), "full supplier identity changed")
        executed = [link for link in links if link["full_call_id"] == before["call_id"]]
        require(len(executed) <= 1, "FULL runnable executed more than once")
        bodies = [other for other in family(calls, "full_body") if executed
                  and other["enter"].get("execution_call_id") == executed[0]["execution_call_id"]]
        if executed:
            require(len(bodies) == 1 and "return" in bodies[0], "executed FULL runnable lacks completed supplier body")
            require(object_id(bodies[0]["enter"]["input_chunk"]) == object_id(before["input_chunk"]), "FULL body consumed a different proto")
        body = bodies[0] if bodies else None
        installed = [other for other in family(calls, "full_status_supplier") if body
                     and other["enter"].get("full_body_call_id") == body["enter"]["call_id"]]
        if body:
            require(len(installed) == 1 and "return" in installed[0], "FULL body lacks status-supplier installation")
            require(object_id(installed[0]["enter"]["chunk"]) == object_id(body["return"]["returned"]), "status supplier installed on wrong chunk")
            if stage_id in stage_exits:
                require(body["return"]["baseline_seq"] < stage_exits[stage_id]["seq"], "baseline completion predates supplier return")
        reports.append({"pos": before["input_chunk"]["pos"], "baseline_stage_id": stage_id,
            "full_call_id": before["call_id"], "full_enter_seq": before["seq"],
            "supplier": suppliers[0]["supplier"] if suppliers else None,
            "native_full_future": call.get("return", {}).get("future"),
            "schedule": schedules[0] if schedules else None, "execution": executed[0] if executed else None,
            "body": body, "status_supplier": installed[0] if installed else None,
            "baseline_stage_exit": stage_exits.get(stage_id), "baseline_stage_entry": stages[stage_id]})
    return reports


def analyze_sidecar(events, terminal_seq, specs, stages, stage_exits):
    calls, types = pair_calls(events, terminal_seq)
    links, pending = match_runnables(calls)
    check_caches(calls)
    return {"event_counts": dict(Counter(row["event"] for row in events)), "object_count": len(types),
        "primary_apis": check_apis(calls, specs), "pumps": check_pumps(calls),
        "accessibility": check_accessibility(calls), "main_runnable_links": links,
        "scheduled_not_observed_executed": pending,
        "full_chains": full_chains(events, calls, links, stages, stage_exits),
        "open_invocations_at_stop": [call for call in calls.values() if "return" not in call],
        "limits": ["Sampled pending-queue contents are weakly consistent and capped at 256 entries; enqueue/run events are uncapped.",
                   "No schedule-return-before-execution order is required across threads.",
                   "Request/phase labels are observation-time context, not causal ownership.",
                   "Open native work at the baseline terminal prefix is retained without draining."]}
