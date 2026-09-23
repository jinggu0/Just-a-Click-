import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import run_concurrency as R  # noqa: E402

CHUNKS = [{'chunk_id': 'a', 'seconds': 100.0, 'reference': '가'},
          {'chunk_id': 'b', 'seconds': 200.0, 'reference': '나'}]


def transcribed(item, status='completed'):
    return {'chunk_id': item['chunk_id'], 'seconds': item['seconds'], 'status': status,
            'rtf': 0.4, 'transcript': f"{item['chunk_id']} 전사", 'peak_private_mib': 1800.0}


class ScheduleTests(unittest.TestCase):
    def test_arrivals_follow_the_audio_and_stop_at_the_limit(self):
        items = R.schedule(CHUNKS, minutes=5)
        self.assertEqual([(i['chunk_id'], i['arrival_seconds'], i['index']) for i in items],
                         [('a', 100.0, 1), ('b', 300.0, 2)])
        self.assertEqual(len(R.schedule(CHUNKS, minutes=8)), 3)

    def test_condition_label_follows_power_and_mode(self):
        ac = {'ac_power': True, 'battery_percent': 90}
        self.assertEqual(R.condition_label(ac, {'ac_mode': 'best_performance'}),
                         'ac-best_performance')
        self.assertEqual(R.condition_label({'ac_power': False}, {'ac_mode': 'best_performance',
                                                                 'dc_mode': 'best_power_efficiency'}),
                         'battery-best_power_efficiency')


class RequestTests(unittest.TestCase):
    def test_draft_payload_fixes_the_output_size(self):
        payload = R.draft_payload('전사문')
        self.assertEqual((payload['max_tokens'], payload['ignore_eos'], payload['temperature']),
                         (300, True, 0))
        self.assertFalse(payload['cache_prompt'])
        self.assertEqual([m['role'] for m in payload['messages']], ['system', 'user'])
        self.assertEqual(payload['messages'][1]['content'], '전사문')

    def test_prompt_cache_is_disabled_by_default(self):
        self.assertEqual(R.CACHE_RAM_MIB, 0)

    def test_draft_result_reads_server_timings(self):
        response = {'choices': [{'message': {'content': '초안 본문'}}],
                    'timings': {'prompt_n': 1900, 'prompt_ms': 3000.0, 'prompt_per_second': 633.3,
                                'predicted_n': 300, 'predicted_ms': 22000.0,
                                'predicted_per_second': 13.6}}
        result = R.draft_result(response)
        self.assertEqual((result['input_tokens'], result['generated_tokens']), (1900, 300))
        self.assertEqual((result['prompt_seconds'], result['generate_seconds']), (3.0, 22.0))
        self.assertEqual((result['prompt_tps'], result['generate_tps']), (633.3, 13.6))
        self.assertEqual(result['output_chars'], 5)


class SampleTests(unittest.TestCase):
    def test_windows_sample_parses_the_powershell_json(self):
        payload = json.dumps({'gpu_shared_bytes': 6 * 1024 * 1024 * 1024, 'discharge_mw': 41198,
                              'remaining_mwh': 45147, 'full_mwh': 52496, 'power_online': 0})
        done = subprocess.CompletedProcess(['powershell'], 0, stdout=payload, stderr='')
        with patch.object(R.subprocess, 'run', return_value=done):
            sample = R.windows_sample(1234)
        self.assertEqual((sample['gpu_shared_mib'], sample['discharge_w']), (6144.0, 41.2))
        self.assertEqual((sample['remaining_mwh'], sample['power_online']), (45147, 0))

    def test_windows_sample_survives_a_failed_call(self):
        with patch.object(R.subprocess, 'run', side_effect=OSError('no powershell')):
            self.assertEqual(R.windows_sample(1234), {})


class SessionTests(unittest.TestCase):
    def session(self, statuses):
        calls = iter(statuses)

        def transcribe(item):
            return transcribed(item, next(calls))

        def draft(job):
            return {'status': 'completed', 'input_tokens': 100, 'prompt_tps': 1.0,
                    'generate_tps': 2.0}

        return R.Session(transcribe, draft, sleep=lambda delay: None)

    def items(self, count, seconds=120.0):
        return [{'chunk_id': f'c{index}', 'seconds': seconds, 'index': index,
                 'arrival_seconds': index * seconds} for index in range(1, count + 1)]

    def test_full_window_and_a_partial_window_at_the_end(self):
        session = self.session(['completed'] * 4)
        chunks, windows = session.run(self.items(4))
        self.assertEqual(len(chunks), 4)
        self.assertEqual([(w['partial'], w['chunks']) for w in windows],
                         [(False, ['c1', 'c2', 'c3']), (True, ['c4'])])
        self.assertEqual(windows[0]['audio_seconds'], 360.0)
        self.assertTrue(all('lag_seconds' in c and 'queued_seconds' in c for c in chunks))

    def test_failed_chunks_stay_out_of_the_draft(self):
        session = self.session(['completed', 'timeout', 'completed', 'completed'])
        chunks, windows = session.run(self.items(4))
        self.assertEqual(sum(1 for c in chunks if c['status'] == 'completed'), 3)
        self.assertEqual([w['chunks'] for w in windows], [['c1', 'c3', 'c4']])

    def test_stop_ends_the_recording_early(self):
        session = self.session(['completed'] * 4)
        chunks, _ = session.run(self.items(4), stop=lambda: True)
        self.assertEqual(chunks, [])


class ReportTests(unittest.TestCase):
    def summary(self):
        return {'runs': [{'stage': 'run', 'condition': 'ac-best_performance', 'stt_threads': 8,
                          'llm_threads': 2,
                          'keepup': {'chunks': 10, 'completed': 10, 'lag_median': 12.0,
                                     'lag_p95': 18.0, 'lag_max': 20.0, 'lag_drift': None,
                                     'ok': True},
                          'windows': {'windows': 2, 'completed': 2, 'seconds_median': 30.0,
                                      'seconds_max': 33.0, 'duty_median': 0.1,
                                      'within_limit': True}}],
                'selected_allocation': {'stt_threads': 8, 'llm_threads': 2},
                'post_recording': {'seconds': 106.3, 'within_target': True},
                'memory': {'total_mib': 9000.0, 'budget_mib': 11980.8, 'gpu_shared_mib': 6000.0,
                           'shared_limit_mib': 8038.4},
                'battery': None, 'verdict': 'feasible_estimate'}

    def test_public_records_drop_transcripts(self):
        records = R.public_records([{'chunk_id': 'a', 'transcript': '내용', 'rtf': 0.4}])
        self.assertEqual(records, [{'chunk_id': 'a', 'rtf': 0.4}])

    def test_render_markdown_lists_runs_and_verdict(self):
        table = R.render_markdown(self.summary())
        self.assertIn('| run | ac-best_performance | 8 | 2 | 10/10 | 12.0 | 18.0 | 20.0 | - | 2/2 '
                      '| 30.0 | 33.0 | 0.100 | 예 | 예 |', table)
        self.assertIn('판정(추정): feasible_estimate', table)

    def test_verdict_requires_every_check(self):
        entry = self.summary()['runs'][0]
        memory = {'fits_budget': True, 'fits_shared_limit': True}
        post = {'within_target': True}
        self.assertEqual(R.verdict(entry, memory, post, False), 'feasible_estimate')
        self.assertEqual(R.verdict(entry, memory, post, True), 'not_judgeable')
        self.assertEqual(R.verdict(entry, dict(memory, fits_shared_limit=False), post, False),
                         'infeasible_estimate')


if __name__ == '__main__':
    unittest.main()
