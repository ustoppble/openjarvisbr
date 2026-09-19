import copy
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import jarvis_trace_report as trace


class AcceptanceTests(unittest.TestCase):
    def complete_run(self):
        script = Path(__file__).resolve().parents[1] / 'docs/testes/roteiro-autonomia.txt'
        phrases = [line.strip() for line in script.read_text().splitlines()
                   if line.strip() and not line.startswith(('#', 'sleep '))]
        names = ['app.open', 'web.open', 'app.open', 'web.open', 'browser.back',
                 'browser.search', 'app.open', 'web.open', 'web.open',
                 'sys.volume', 'sys.volume']
        turns = []
        for n, phrase in enumerate(phrases):
            turn = trace.Turn(started=f'2026-09-19T03:00:{n:02d}.000', user=[phrase])
            if n < len(names):
                turn.tools[str(n)] = trace.Tool(id=str(n), name=names[n], ok=True,
                                                origem='reflexo' if n == 8 else 'modelo')
            else:
                turn.reflex.append((turn.started, 'Nothing', -1))
            turns.append(turn)
        return turns

    def test_complete_run_passes(self):
        self.assertTrue(trace.aceitacao(self.complete_run())[1])

    def test_browser_goto_is_a_valid_navigation(self):
        turns = self.complete_run()
        for n in [1, 3, 7, 8]:
            turns[n].tools[str(n)].name = 'browser.goto'
        self.assertTrue(trace.aceitacao(turns)[1])

    def test_empty_log_fails(self):
        self.assertFalse(trace.aceitacao([])[1])

    def test_missing_required_turn_fails(self):
        for n in range(len(self.complete_run())):
            with self.subTest(turn=n):
                turns = self.complete_run()
                del turns[n]
                self.assertFalse(trace.aceitacao(turns)[1])

    def test_action_must_finish_successfully_with_expected_tool(self):
        for state in ['missing', 'pending', 'failed', 'wrong_tool']:
            with self.subTest(state=state):
                turns = self.complete_run()
                tool = next(iter(turns[0].tools.values()))
                if state == 'missing':
                    turns[0].tools.clear()
                elif state == 'wrong_tool':
                    tool.name = 'volume.set'
                else:
                    tool.ok = None if state == 'pending' else False
                self.assertFalse(trace.aceitacao(turns)[1])

    def test_second_globo_must_succeed_via_reflex(self):
        for origin, ok in [('modelo', True), ('reflexo', False), ('reflexo', None)]:
            with self.subTest(origin=origin, ok=ok):
                turns = self.complete_run()
                tool = next(iter(turns[8].tools.values()))
                tool.origem, tool.ok = origin, ok
                self.assertFalse(trace.aceitacao(turns)[1])

    def test_time_question_must_skip_judge(self):
        for decisions in [[], [('2026-09-19T03:00:11.000', 'Nothing', 200)]]:
            with self.subTest(decisions=decisions):
                turns = self.complete_run()
                turns[-1].reflex = decisions
                self.assertFalse(trace.aceitacao(turns)[1])

    def test_runner_error_fails_even_with_complete_trace(self):
        self.assertFalse(trace.aceitacao(self.complete_run(), runner_exit=3)[1])

    def test_duplicates_and_refusals_still_fail(self):
        turns = self.complete_run()
        duplicate = copy.copy(turns[0].tools['0'])
        duplicate.id = 'again'
        turns[0].tools['again'] = duplicate
        self.assertFalse(trace.aceitacao(turns)[1])
        turns = self.complete_run()
        turns[0].model = ['Não consigo abrir.']
        self.assertFalse(trace.aceitacao(turns)[1])

    def test_parser_distinguishes_skipped_judge_from_zero_latency_call(self):
        prefix = '2026-09-19T03:00:00.000Z DEBUG openjarvisbr_core::reflex: '
        for skipped, expected in [('true', -1), ('false', 0)]:
            turns = trace.parse([prefix + 'reflexo decision=Nothing latency_ms=0 '
                                 f'skipped_judge={skipped} fala="que horas são?"'], None)
            self.assertEqual(turns[0].reflex[0][2], expected)

    def test_report_masks_addresses_and_bearer(self):
        address = '.'.join(str(n) for n in [192, 0, 2, 7]) + ':8123'
        host = 'example' + '.test'
        secret = 's' * 40
        turns = self.complete_run()
        turns[0].model = ['https' + '://' + host + '/?token=' + secret,
                          ' Bearer ' + secret + ' servidor=' + address]
        rendered = trace.report(turns)
        for sensitive in [address, host, secret]:
            self.assertNotIn(sensitive, rendered)


class RunnerTests(unittest.TestCase):
    def test_collects_stdout_and_stderr_and_forwards_device_argument(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory(prefix='jarvis-runner-test-') as directory:
            root = Path(directory)
            (root / 'tools').mkdir()
            (root / 'docs/superpowers/briefs').mkdir(parents=True)
            (root / 'target/release').mkdir(parents=True)
            for name in ['rodada_final.sh', 'jarvis_trace_report.py']:
                shutil.copy2(source / name, root / 'tools' / name)
            runner = root / 'target/release/jarvis'
            runner.write_text('#!/bin/bash\nprintf "trace em stdout\\n"\n'
                              'printf "erro em stderr\\n" >&2\n'
                              'printf "%s\\n" "$@" > "$ARGUMENTS_FILE"\nexit 3\n')
            runner.chmod(0o755)
            arguments = root / 'arguments.txt'
            env = dict(os.environ, CARGO_TARGET_DIR=str(root / 'target'),
                       ARGUMENTS_FILE=str(arguments))
            name = root.name
            # Exercita o script real, sem compilar nem fechar aplicativos reais.
            shell = ('cargo() { return 0; }; pkill() { return 0; }; '
                     'git() { printf "fixture\\n"; }; export -f cargo pkill git; '
                     'bash "$@"')
            result = subprocess.run(['bash', '-c', shell, 'test',
                                     str(root / 'tools/rodada_final.sh'), 'roteiro.txt', name,
                                     '--device-in', 'MacBook Pro Microphone'],
                                    env=env, capture_output=True, text=True)
            artifacts = list(Path('/tmp').glob(f'jarvis-{name}-*'))
            try:
                self.assertEqual(result.returncode, 1)
                logs = [p for p in artifacts if p.suffix == '.log']
                self.assertEqual(len(logs), 1)
                self.assertIn('trace em stdout', logs[0].read_text())
                self.assertIn('erro em stderr', logs[0].read_text())
                self.assertIn('--device-in\nMacBook Pro Microphone', arguments.read_text())
                report = next((root / 'docs/superpowers/briefs').glob('*.md')).read_text()
                self.assertIn('runner exit = 3 (esperado 0)', report)
                self.assertIn('NÃO PASSOU', report)
            finally:
                for artifact in artifacts:
                    artifact.unlink()


if __name__ == '__main__':
    unittest.main()
