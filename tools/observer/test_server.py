#!/usr/bin/env python3
from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tools.observer import server


ROUND_ONE = ROOT / "docs" / "testes" / "rodada1-trace.log"


class ObserverTest(unittest.TestCase):
    def test_observe_reuses_canonical_round_parser(self) -> None:
        summary = server.observe_session(ROUND_ONE)

        self.assertEqual(summary["turns"], 12)
        self.assertEqual(summary["duplicates"], 2)
        self.assertEqual(summary["refusals"], 1)
        self.assertEqual(summary["jev_latency_ms"]["samples"], 11)
        self.assertGreater(summary["jev_without_action"]["turns"], 0)

    def test_propose_tasks_groups_occurrences_by_failure_pattern(self) -> None:
        tasks = server.propose_tasks(ROUND_ONE)
        duplicate = next(task for task in tasks if task["pattern"] == "action:duplicada")

        self.assertEqual(duplicate["occurrences"], 2)
        self.assertEqual(len(duplicate["evidence"]), 2)
        self.assertTrue(duplicate["o_que"])
        self.assertTrue(duplicate["por_que"])
        self.assertTrue(duplicate["como_confirmo"])
        self.assertTrue(all(" linha " in item for item in duplicate["evidence"]))

    def test_unstructured_app_failure_inherits_previous_timestamp(self) -> None:
        trace = (
            "2026-09-18T19:21:57.146819Z DEBUG openjarvisbr_core::reflex: "
            "reflexo decision=Nothing latency_ms=359\n"
            "Unable to find application named 'Calculadora'\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "jarvis-trace.log"
            path.write_text(trace, encoding="utf-8")

            failures = server.list_failures(path)

        app_failure = next(
            item
            for item in failures
            if item["pattern"] == "app.open:aplicativo_nao_encontrado"
        )
        self.assertEqual(app_failure["line_number"], 2)
        self.assertEqual(app_failure["timestamp"], "2026-09-18T19:21:57.146819Z")
        self.assertIn("Calculadora", app_failure["evidence"])

    def test_failure_output_never_exposes_credentials_or_addresses(self) -> None:
        credential = "x" * 40
        address = ".".join(("private", "invalid"))
        trace = (
            "2026-09-18T19:21:42.000000Z WARN openjarvisbr_core::engine: "
            f"tool falhou id=call_1 ferramenta=net.request origem=\"modelo\" ms=1 "
            f"erro=token={credential} host={address}\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "jarvis-trace.log"
            path.write_text(trace, encoding="utf-8")
            output = json.dumps(server.list_failures(path), ensure_ascii=False)

        self.assertNotIn(credential, output)
        self.assertNotIn(address, output)
        self.assertNotIn("token=", output)
        self.assertNotIn("host=", output)

    def test_events_log_enriches_a_trace_without_reflex_metrics(self) -> None:
        trace = (
            "2026-09-18T19:21:42.000000Z DEBUG openjarvisbr_core::engine: "
            'usuário disse texto="teste"\n'
        )
        events = (
            "     100ms  reflex_act     reflex-1 app.open Teste 321ms\n"
            "     200ms  reflex_learn   app.open Teste\n"
            "     300ms  tool_dedup_reflex call_1\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            trace_path = Path(directory) / "jarvis-trace.log"
            events_path = Path(directory) / "events.log"
            trace_path.write_text(trace, encoding="utf-8")
            events_path.write_text(events, encoding="utf-8")

            summary = server.observe_session(trace_path, events_path)

        self.assertEqual(summary["reflex_actions"], 1)
        self.assertEqual(summary["reflex_learns"], 1)
        self.assertEqual(summary["deduplicated_by_reflex"], 1)
        self.assertEqual(summary["jev_latency_ms"]["average"], 321)

    def test_mcp_lists_and_calls_the_three_tools(self) -> None:
        requests = [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": "2025-06-18"},
            },
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "observe_session",
                    "arguments": {"trace_path": str(ROUND_ONE)},
                },
            },
        ]
        reader = io.StringIO("".join(json.dumps(request) + "\n" for request in requests))
        writer = io.StringIO()

        server.serve(reader, writer)

        responses = [json.loads(line) for line in writer.getvalue().splitlines()]
        names = [tool["name"] for tool in responses[1]["result"]["tools"]]
        self.assertEqual(names, ["observe_session", "list_failures", "propose_tasks"])
        self.assertEqual(responses[2]["result"]["structuredContent"]["turns"], 12)

    def test_registration_preserves_existing_config_and_is_idempotent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.toml"
            config.write_text('api_key = "***"\n[tools]\nenabled = true\n', encoding="utf-8")

            self.assertEqual(server.register_server(config), "added")
            self.assertEqual(server.register_server(config), "already_present")
            written = config.read_text(encoding="utf-8")

        self.assertIn('api_key = "***"', written)
        self.assertEqual(written.count('name = "observer"'), 1)


if __name__ == "__main__":
    unittest.main()
