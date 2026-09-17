from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import stt_metrics as M  # noqa: E402

STDERR = """system_info: n_threads = 4 / 8 | WHISPER : VITISAI = 0 |
main: processing '<repo>/a.wav' (199680 samples, 12.5 sec), 4 threads, 1 processors, 5 beams + best of 5, lang = ko, task = transcribe, timestamps = 1 ...
whisper_print_timings:     load time =   269.30 ms
whisper_print_timings:     fallbacks =   1 p /   2 h
whisper_print_timings:      mel time =    15.96 ms
whisper_print_timings:   encode time =  4586.51 ms /     1 runs (  4586.51 ms per run)
whisper_print_timings:    total time =  7669.65 ms
"""


class NormalizationTests(unittest.TestCase):
    def test_normalize_drops_spaces_punctuation_and_case(self):
        self.assertEqual(M.normalize('비슈케크(Bishkek)는, 15m 이다.'), '비슈케크bishkek는15m이다')

    def test_nfkc_composes_jamo(self):
        self.assertEqual(M.normalize('한'), '한')

    def test_hangul_only_ignores_latin_and_digits(self):
        self.assertEqual(M.hangul_only('비슈케크(Bishkek) 2011년'), '비슈케크년')


class ErrorRateTests(unittest.TestCase):
    def test_edit_distance(self):
        self.assertEqual(M.edit_distance('kitten', 'sitting'), 3)
        self.assertEqual(M.edit_distance('', 'abc'), 3)
        self.assertEqual(M.edit_distance('같다', '같다'), 0)

    def test_char_errors_counts_both_views(self):
        errors = M.char_errors('다리 밑 (Bridge) 간격', '다리미 간격')
        self.assertEqual((errors['ref_chars'], errors['hyp_chars']), (11, 5))
        self.assertEqual(errors['errors'], 7)
        self.assertEqual((errors['ref_hangul'], errors['errors_hangul']), (5, 1))

    def test_error_rate_handles_empty_reference(self):
        self.assertEqual(M.error_rate(1, 3), 0.3333)
        self.assertIsNone(M.error_rate(0, 0))

    def test_anomaly_flags(self):
        self.assertFalse(M.suspected_anomaly(
            {'ref_chars': 10, 'hyp_chars': 10, 'errors': 5}))
        self.assertTrue(M.suspected_anomaly({'ref_chars': 10, 'hyp_chars': 10, 'errors': 6}))
        self.assertTrue(M.suspected_anomaly({'ref_chars': 10, 'hyp_chars': 20, 'errors': 0}))
        self.assertTrue(M.suspected_anomaly({'ref_chars': 0, 'hyp_chars': 2, 'errors': 2}))


class TimingTests(unittest.TestCase):
    def test_parse_timings(self):
        timings = M.parse_timings(STDERR)
        self.assertEqual((timings['load_ms'], timings['total_ms']), (269.30, 7669.65))
        self.assertEqual(timings['fallbacks'], 3)
        self.assertEqual(timings['decoding'], {'threads': 4, 'processors': 1, 'beams': 5,
                                               'best_of': 5, 'language': 'ko'})

    def test_missing_timings_rejected(self):
        with self.assertRaises(ValueError):
            M.parse_timings('whisper_print_timings:     load time =   1.00 ms')

    def test_nearest_rank_percentile(self):
        values = [float(v) for v in range(1, 21)]
        self.assertEqual(M.percentile(values, 0.95), 19.0)
        self.assertEqual(M.percentile([0.3], 0.95), 0.3)
        with self.assertRaises(ValueError):
            M.percentile([], 0.5)


if __name__ == '__main__':
    unittest.main()
