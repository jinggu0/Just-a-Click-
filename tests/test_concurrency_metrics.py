from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import concurrency_metrics as M  # noqa: E402


def chunk(index, lag, arrival=None, status='completed', queued=1.0, rtf=0.4):
    return {'chunk_id': f'c{index:03d}', 'seconds': 25.0, 'status': status,
            'arrival_seconds': arrival if arrival is not None else index * 25.0,
            'lag_seconds': lag, 'queued_seconds': queued, 'rtf': rtf}


def window(seconds, wait=0.0, status='completed'):
    return {'status': status, 'seconds': seconds, 'wait_seconds': wait, 'input_tokens': 2000,
            'prompt_tps': 500.0, 'generate_tps': 12.0}


class WindowBoundaryTests(unittest.TestCase):
    def test_window_closes_once_the_audio_reaches_five_minutes(self):
        pending = [{'seconds': 100.0}, {'seconds': 120.0}, {'seconds': 100.0}]
        self.assertEqual(len(M.closed_window(pending)), 3)
        self.assertEqual(len(M.closed_window([{'seconds': 200.0}, {'seconds': 150.0}])), 2)
        self.assertIsNone(M.closed_window([{'seconds': 100.0}]))


class KeepupTests(unittest.TestCase):
    def test_short_lags_keep_up(self):
        result = M.keepup([chunk(i, 12.0) for i in range(20)], minutes=10)
        self.assertEqual((result['lag_p95'], result['lag_max'], result['failures']), (12.0, 12.0, 0))
        self.assertIsNone(result['lag_drift'])
        self.assertTrue(result['ok'])

    def test_long_tail_or_growing_queue_fails(self):
        slow = [chunk(i, 12.0) for i in range(19)] + [chunk(19, 70.0)]
        self.assertFalse(M.keepup(slow, minutes=10)['ok'])
        early = [chunk(i, 10.0, arrival=i * 60.0) for i in range(30)]
        late = [chunk(30 + i, 25.0, arrival=5400.0 + i * 60.0) for i in range(30)]
        drifting = M.keepup(early + late, minutes=120)
        self.assertEqual(drifting['lag_drift'], 15.0)
        self.assertFalse(drifting['ok'])

    def test_failed_chunks_are_counted_but_lags_come_from_completed_ones(self):
        result = M.keepup([chunk(0, 10.0), chunk(1, 0.0, status='timeout')], minutes=10)
        self.assertEqual((result['completed'], result['failures']), (1, 1))
        self.assertEqual(M.keepup([chunk(0, 0.0, status='failed')], minutes=10)['ok'], False)


class WindowStatsTests(unittest.TestCase):
    def test_window_statistics_and_limit(self):
        stats = M.window_stats([window(30.0), window(40.0, wait=5.0), window(0.0, status='failed')])
        self.assertEqual((stats['windows'], stats['completed'], stats['failures']), (3, 2, 1))
        self.assertEqual((stats['seconds_median'], stats['seconds_max']), (35.0, 40.0))
        self.assertEqual((stats['wait_seconds_max'], stats['duty_median']), (5.0, 0.117))
        self.assertTrue(stats['within_limit'])
        self.assertFalse(M.window_stats([window(200.0)])['within_limit'])
        self.assertFalse(M.window_stats([])['within_limit'])

    def test_post_recording_estimate_uses_the_slowest_window(self):
        self.assertEqual(M.post_recording_estimate(60.0),
                         {'seconds': 160.3, 'target_seconds': 300.0, 'backlog_windows': 2,
                          'final_integration_seconds': 40.3, 'within_target': True})
        self.assertFalse(M.post_recording_estimate(150.0)['within_target'])


class MemoryTests(unittest.TestCase):
    def test_memory_budget_checks_shared_limit_and_total(self):
        result = M.memory_budget(2000.0, 1500.0, 7000.0)
        self.assertEqual(result['total_mib'], 10500.0)
        self.assertTrue(result['fits_budget'] and result['fits_shared_limit'])
        self.assertFalse(M.memory_budget(2000.0, 1500.0, 8500.0)['fits_shared_limit'])
        self.assertFalse(M.memory_budget(3000.0, 4000.0, 7000.0)['fits_budget'])

    def test_double_counted_memory_is_counted_once(self):
        result = M.memory_budget(1000.0, 7500.0, 7000.0, double_counted=True)
        self.assertEqual(result['total_mib'], 8500.0)


class BatteryTests(unittest.TestCase):
    def test_battery_drain_per_hour(self):
        samples = [{'seconds': 0.0, 'remaining_mwh': 50000}, {'seconds': 3600.0, 'remaining_mwh': 35000}]
        self.assertEqual(M.battery_drain(samples, 50000),
                         {'hours': 1.0, 'used_wh': 15.0, 'percent_per_hour': 30.0,
                          'full_charge_hours': 3.3})
        self.assertIsNone(M.battery_drain(samples[:1], 50000))
        self.assertIsNone(M.battery_drain(
            [{'seconds': 0.0, 'remaining_mwh': 50000}, {'seconds': 60.0, 'remaining_mwh': 50000}],
            50000))


class SelectionTests(unittest.TestCase):
    def entry(self, stt, llm, median, lag_p95, ok=True):
        return {'stt_threads': stt, 'llm_threads': llm,
                'keepup': {'ok': ok, 'lag_p95': lag_p95},
                'windows': {'seconds_median': median}}

    def test_fastest_draft_wins_and_near_ties_go_to_the_lower_lag(self):
        entries = [self.entry(8, 4, 40.0, 25.0), self.entry(8, 2, 42.0, 12.0),
                   self.entry(6, 2, 30.0, 11.0, ok=False)]
        self.assertEqual(M.select_allocation(entries), {'stt_threads': 8, 'llm_threads': 2})
        entries[1]['windows']['seconds_median'] = 60.0
        self.assertEqual(M.select_allocation(entries), {'stt_threads': 8, 'llm_threads': 4})
        self.assertIsNone(M.select_allocation([self.entry(8, 4, 40.0, 25.0, ok=False)]))


class SuspendTests(unittest.TestCase):
    def test_sample_gap_marks_a_suspended_run(self):
        self.assertFalse(M.suspended([0.0, 5.0, 10.0, 15.0]))
        self.assertTrue(M.suspended([0.0, 5.0, 400.0]))


if __name__ == '__main__':
    unittest.main()
