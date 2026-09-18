import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import run_stt_bench as S  # noqa: E402

STDERR = ("main: processing 'x.wav' (480000 samples, 30.0 sec), 8 threads, 1 processors, "
          "5 beams + best of 5, lang = ko, task = transcribe, timestamps = 1 ...\n"
          "whisper_print_timings:     load time =   500.00 ms\n"
          "whisper_print_timings:     fallbacks =   0 p /   1 h\n"
          "whisper_print_timings:    total time =  6500.00 ms\n")
CHUNK = {'chunk_id': 'fleurs-ko-001', 'seconds': 20.0, 'reference': '다리 밑 간격'}


def record(chunk_id, rtf, errors=0, ref=10, status='completed', suspect=False):
    return {'chunk_id': chunk_id, 'seconds': 20.0, 'status': status, 'rtf': rtf,
            'processing_seconds': rtf * 20.0, 'load_seconds': 0.5, 'fallbacks': 1,
            'errors': errors, 'ref_chars': ref, 'hyp_chars': ref, 'errors_hangul': errors,
            'ref_hangul': ref, 'anomaly_suspect': suspect}


class CommandTests(unittest.TestCase):
    def test_screen_grid(self):
        self.assertEqual([(c['build'], c['threads']) for c in S.screen_configs()],
                         [('cpu', 4), ('cpu', 8), ('blas', 4), ('blas', 8)])

    def test_cli_command_defaults_to_no_timestamps(self):
        args = (Path('w/whisper-cli.exe'), Path('m.bin'), Path('c.wav'), 8, Path('o/c'))
        base = ['-m', 'm.bin', '-f', 'c.wav', '-l', 'ko', '-t', '8', '-oj', '-of', str(Path('o/c'))]
        self.assertEqual(S.cli_command(*args)[1:], base + ['-nt'])
        self.assertEqual(S.cli_command(*args, timestamps=True)[1:], base)

    def test_run_folder_keeps_stages_apart(self):
        out = Path('out')
        screen = S.run_folder(out, 'screen', 'turbo', 'blas', 8, False)
        self.assertEqual(screen, out / 'screen-turbo-blas-t8-nt')
        self.assertNotEqual(screen, S.run_folder(out, 'models', 'turbo', 'blas', 8, False))
        self.assertNotEqual(S.run_folder(out, 'timestamp_check', 'turbo', 'blas', 8, True),
                            S.run_folder(out, 'timestamp_check', 'turbo', 'blas', 8, False))

    def test_transcript_text_joins_segments(self):
        result = {'transcription': [{'text': ' 다리 밑'}, {'text': ' 간격 '}]}
        self.assertEqual(S.transcript_text(result), '다리 밑 간격')


class RunChunkTests(unittest.TestCase):
    def test_completed_chunk_measures_processing_without_load(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / 'fleurs-ko-001'
            Path(f'{prefix}.json').write_text(
                json.dumps({'transcription': [{'text': ' 다리미 간격'}]}), 'utf-8')
            done = subprocess.CompletedProcess(['x'], 0, stdout='', stderr=STDERR)
            with patch.object(S.subprocess, 'run', return_value=done):
                result = S.run_chunk(['x'], prefix, CHUNK)
        self.assertEqual(result['status'], 'completed')
        self.assertEqual((result['load_seconds'], result['processing_seconds']), (0.5, 6.0))
        self.assertEqual(result['rtf'], 0.3)
        self.assertEqual((result['errors'], result['ref_chars'], result['fallbacks']), (1, 5, 1))
        self.assertEqual(result['decoding']['beams'], 5)
        self.assertFalse(result['anomaly_suspect'])

    def test_failed_exit_hides_paths(self):
        done = subprocess.CompletedProcess(['x'], 3, stdout='', stderr=f'error in {S.ROOT}\\m.bin')
        with patch.object(S.subprocess, 'run', return_value=done):
            result = S.run_chunk(['x'], Path('missing'), CHUNK)
        self.assertEqual((result['status'], result['returncode']), ('failed', 3))
        self.assertIn('<repo>\\m.bin', result['error'])
        self.assertNotIn(str(S.ROOT), result['error'])

    def test_missing_output_or_timeout(self):
        done = subprocess.CompletedProcess(['x'], 0, stdout='', stderr=STDERR)
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(S.subprocess, 'run', return_value=done):
            result = S.run_chunk(['x'], Path(directory) / 'none', CHUNK)
        self.assertEqual(result['status'], 'failed')
        with patch.object(S.subprocess, 'run', side_effect=subprocess.TimeoutExpired(['x'], 1)):
            self.assertEqual(S.run_chunk(['x'], Path('p'), CHUNK)['status'], 'timeout')


class SummaryTests(unittest.TestCase):
    def test_realtime_needs_median_p95_and_few_failures(self):
        records = [record(f'c{i}', 0.3, errors=1) for i in range(19)] + [record('c19', 0.9)]
        summary = S.summarize(records, 20)
        self.assertEqual((summary['rtf_median'], summary['rtf_p95'], summary['rtf_max']),
                         (0.3, 0.3, 0.9))
        self.assertEqual((summary['cer'], summary['rtf_total']), (0.095, 0.33))
        self.assertTrue(summary['realtime_ok'])
        slow = [record(f'c{i}', 0.3) for i in range(18)] + [record('a', 0.9), record('b', 0.9)]
        self.assertFalse(S.summarize(slow, 20)['realtime_ok'])
        failed = [record(f'c{i}', 0.3) for i in range(17)]
        self.assertFalse(S.summarize(failed, 20)['judgeable'])
        self.assertFalse(S.summarize([], 5)['realtime_ok'])

    def test_anomaly_suspects_listed(self):
        summary = S.summarize([record('c1', 0.2, suspect=True), record('c2', 0.2)], 2)
        self.assertEqual(summary['anomaly_suspects'], ['c1'])


class SelectionTests(unittest.TestCase):
    def entry(self, model, cer, rtf, ok=True, completed=10, expected=10, judgeable=True):
        return {'model': model, 'build': 'cpu', 'threads': 4, 'timestamps': False,
                'summary': {'cer': cer, 'rtf_median': rtf, 'realtime_ok': ok,
                            'completed': completed, 'expected_chunks': expected,
                            'judgeable': judgeable}}

    def test_select_config_uses_fastest_complete_run(self):
        fast_incomplete = dict(self.entry('m', 0.1, 0.1, completed=9), build='blas', threads=8)
        slow = self.entry('m', 0.1, 0.4)
        faster = dict(self.entry('m', 0.1, 0.3), threads=8)
        self.assertEqual(S.select_config([fast_incomplete, slow, faster]),
                         {'build': 'cpu', 'threads': 8})
        with self.assertRaises(ValueError):
            S.select_config([fast_incomplete])

    def test_select_model_prefers_speed_within_half_point(self):
        entries = [self.entry('large', 0.080, 0.45), self.entry('turbo', 0.084, 0.20),
                   self.entry('small', 0.150, 0.05), self.entry('huge', 0.050, 0.90, ok=False)]
        self.assertEqual(S.select_model(entries), 'turbo')
        entries[1]['summary']['cer'] = 0.086
        self.assertEqual(S.select_model(entries), 'large')
        self.assertIsNone(S.select_model([self.entry('huge', 0.05, 0.9, ok=False)]))

    def test_top_models_ranks_judgeable_by_cer(self):
        entries = [self.entry('a', 0.10, 0.1), self.entry('b', 0.05, 0.9, ok=False),
                   self.entry('c', 0.01, 0.1, judgeable=False), self.entry('d', 0.07, 0.2),
                   self.entry('e', None, 0.2)]
        self.assertEqual(S.top_models(entries), ['b', 'd'])

    def test_render_markdown_lists_runs(self):
        run = dict(self.entry('turbo', 0.084, 0.2), summary=S.summarize([record('c1', 0.2)], 1))
        summary = {'screen': [], 'models': [run],
                   'timestamp_check': [dict(run, timestamps=True)],
                   'selected_config': {'build': 'cpu', 'threads': 4}, 'selected_model': 'turbo'}
        table = S.render_markdown(summary)
        self.assertIn('| models | turbo | cpu | 4 | -nt | 1/1 |', table)
        self.assertIn('| timestamp_check | turbo | cpu | 4 | 사용 | 1/1 |', table)
        self.assertIn('1차 후보 모델(추정): turbo', table)


if __name__ == '__main__':
    unittest.main()
