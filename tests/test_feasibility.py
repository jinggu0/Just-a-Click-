from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import feasibility as F  # noqa: E402
import llama_bench_results as L  # noqa: E402


def bench(runtime='vulkan', test='tg', depth=0, rate=5.0, ubatch=512, threads=8, fa=1):
    tokens = {'n_prompt': 512, 'n_gen': 0} if test == 'pp' else {'n_prompt': 0, 'n_gen': 128}
    return L.normalize({
        'build_commit': '0a8b29a60', 'build_number': 10994, 'backends': 'Vulkan',
        'cpu_info': 'cpu', 'gpu_info': 'gpu',
        'model_filename': 'Qwen3-8B-Q5_K_M.gguf', 'model_type': 'qwen3 8B Q5_K - Medium',
        'n_batch': 2048, 'n_ubatch': ubatch, 'n_threads': threads, 'n_gpu_layers': 99,
        'flash_attn': fa, 'n_depth': depth, 'test_time': '2026-09-17T00:00:00Z',
        'samples_ts': [rate], **tokens}, runtime)


class CurveMathTests(unittest.TestCase):
    def test_rate_interpolates_and_clamps(self):
        points = [[0, 100.0], [2048, 50.0], [8192, 20.0]]
        self.assertEqual(F.rate_at(points, -5), 100.0)
        self.assertEqual(F.rate_at(points, 1024), 75.0)
        self.assertEqual(F.rate_at(points, 5120), 35.0)
        self.assertEqual(F.rate_at(points, 10000), 20.0)

    def test_seconds_integrates_declining_rate(self):
        self.assertAlmostEqual(F.seconds([[0, 10.0]], 0, 100), 10.0)
        # Exact integral of 1 / (100 - 0.05 d) over 0..1000 is 20 ln 2.
        self.assertAlmostEqual(F.seconds([[0, 100.0], [1000, 50.0]], 0, 1000), 13.863, delta=0.01)

    def test_prompt_curve_prefers_covering_then_faster_ubatch(self):
        records = [bench(test='pp', depth=0, rate=100.0, ubatch=512),
                   bench(test='pp', depth=2048, rate=60.0, ubatch=512),
                   bench(test='pp', depth=0, rate=80.0, ubatch=128),
                   bench(test='pp', depth=2048, rate=50.0, ubatch=128),
                   bench(test='pp', depth=8192, rate=20.0, ubatch=128)]
        deep = F.prompt_curve(records, 'vulkan', 8, 'on', need_depth=3000)
        self.assertEqual((deep['ubatch'], deep['extrapolated']), (128, False))
        short = F.prompt_curve(records, 'vulkan', 8, 'on', need_depth=2000)
        self.assertEqual((short['ubatch'], short['points']), (512, [[0, 100.0], [2048, 60.0]]))


class EstimateTests(unittest.TestCase):
    def test_flat_rates_match_hand_calculation(self):
        scenarios = F.estimate([[0, 100.0]], [[0, 5.0]])
        first = scenarios[0]
        self.assertEqual((first['lecture_hours'], first['syllables_per_second']), (2, 2.5))
        self.assertEqual(first['window_input_tokens'], 1575)
        self.assertEqual(first['window_seconds'], 75.8)
        self.assertEqual(first['post_recording_seconds'], 261.5)
        self.assertEqual(first['library_resummary_seconds'], 1928.0)
        self.assertEqual(first['required_tg_tps_post'], 4.19)
        self.assertEqual(first['required_tg_tps_resummary'], 2.38)
        self.assertEqual(F.verdict(scenarios, [2, 3]), 'feasible_estimate')
        four_hours = [s for s in scenarios if s['lecture_hours'] == 4]
        self.assertFalse(any(s['library_resummary_within_target'] for s in four_hours))

    def test_verdict_degrades_with_slower_generation(self):
        borderline = F.estimate([[0, 100.0]], [[0, 4.2]])
        self.assertEqual(F.verdict(borderline, [2, 3]), 'borderline_estimate')
        slow = F.estimate([[0, 100.0]], [[0, 5.0]], tg_factor=0.5)
        self.assertEqual(F.verdict(slow, [2, 3]), 'infeasible_estimate')

    def test_prompt_time_alone_can_exceed_budget(self):
        self.assertIsNone(F.required_rate(100, 300, 301))

    def test_estimate_from_records_reports_selection(self):
        records = [bench(test='tg', depth=0, rate=5.0), bench(test='tg', depth=8192, rate=4.0),
                   bench(test='pp', depth=0, rate=100.0), bench(test='pp', depth=8192, rate=40.0),
                   bench(runtime='cpu', test='tg', rate=50.0)]
        result = F.estimate_from_records(records, {'threads': 8, 'flash_attn': 'on'})
        self.assertEqual(result['generation_curve'], [[0, 5.0], [8192, 4.0]])
        self.assertEqual(result['prompt_curve']['ubatch'], 512)
        self.assertIn(result['verdict'], {'feasible_estimate', 'borderline_estimate',
                                          'infeasible_estimate'})
        table = F.render_markdown(records, result)
        self.assertIn('판정(추정)', table)
        self.assertIn('| vulkan | 8 | on | 512 | pp512 | 8192 | 40.00 | 40.00~40.00 |', table)


class ServerFactorTests(unittest.TestCase):
    def test_factor_compares_at_same_depth(self):
        report = {'response_format': 'schema', 'runs': [
            {'timings': {'prompt_n': 926, 'predicted_n': 329, 'predicted_per_second': 2.92}}]}
        factor = F.server_factor([report], [[0, 5.0], [2048, 4.0]])
        self.assertAlmostEqual(factor, 2.92 / (5.0 - 1090.5 / 2048), places=6)

    def test_unconstrained_reports_rejected(self):
        with self.assertRaises(ValueError):
            F.server_factor([{'response_format': 'none', 'runs': []}], [[0, 5.0]])
        with self.assertRaises(ValueError):
            F.server_factor([{'response_format': 'schema', 'runs': []}], [[0, 5.0]])


if __name__ == '__main__':
    unittest.main()
