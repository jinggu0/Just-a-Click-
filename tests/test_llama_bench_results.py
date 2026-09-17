from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import llama_bench_results as L  # noqa: E402


def raw(**overrides):
    record = {'build_commit': '0a8b29a60', 'build_number': 10994, 'backends': 'Vulkan',
              'cpu_info': 'Intel(R) Core(TM) Ultra 7 258V',
              'gpu_info': 'Intel(R) Arc(TM) 140V GPU (16GB)',
              'model_filename': 'C:/work/models/Qwen3-8B-Q5_K_M.gguf',
              'model_type': 'qwen3 8B Q5_K - Medium', 'n_batch': 2048, 'n_ubatch': 512,
              'n_threads': 8, 'n_gpu_layers': 99, 'flash_attn': 1, 'n_prompt': 0,
              'n_gen': 128, 'n_depth': 0, 'test_time': '2026-09-17T05:09:24Z',
              'samples_ts': [4.0, 6.0, 5.0]}
    record.update(overrides)
    return record


def record(**overrides):
    runtime = overrides.pop('runtime', 'vulkan')
    return L.normalize(raw(**overrides), runtime)


class ParseTests(unittest.TestCase):
    def test_skips_log_noise_and_counts_partial_line(self):
        text = 'ggml_vulkan: device lost\n{"n_prompt": 8}\n{"n_prompt": 8, "n_g\n'
        records, malformed = L.parse_jsonl(text)
        self.assertEqual(records, [{'n_prompt': 8}])
        self.assertEqual(malformed, 1)

    def test_normalize_prompt_record_without_local_path(self):
        value = L.normalize(raw(n_prompt=512, n_gen=0, flash_attn=-1,
                                model_filename='C:\\Users\\name\\m\\Qwen3-8B-Q5_K_M.gguf'), 'cpu')
        self.assertEqual((value['test'], value['tokens'], value['depth']), ('pp', 512, 0))
        self.assertEqual((value['median_tps'], value['min_tps'], value['max_tps']), (5.0, 4.0, 6.0))
        self.assertEqual((value['runtime'], value['flash_attn']), ('cpu', 'auto'))
        self.assertEqual(value['model'], 'Qwen3-8B-Q5_K_M.gguf')

    def test_combined_or_empty_records_rejected(self):
        with self.assertRaises(ValueError):
            L.normalize(raw(n_prompt=512, n_gen=128), 'vulkan')
        with self.assertRaises(ValueError):
            L.normalize(raw(samples_ts=[]), 'vulkan')


class SelectionTests(unittest.TestCase):
    def test_generation_speed_wins_then_prompt_speed(self):
        records = [
            record(n_threads=4, flash_attn=1, samples_ts=[5.0]),
            record(n_threads=8, flash_attn=1, samples_ts=[5.0]),
            record(n_threads=4, flash_attn=1, n_prompt=512, n_gen=0, samples_ts=[80.0]),
            record(n_threads=8, flash_attn=1, n_prompt=512, n_gen=0, samples_ts=[90.0]),
            record(n_threads=8, flash_attn=0, samples_ts=[4.0]),
            record(n_threads=8, flash_attn=0, n_depth=2048, samples_ts=[9.0]),
            record(runtime='cpu', n_threads=4, samples_ts=[9.0])]
        self.assertEqual(L.select_best(records), {'threads': 8, 'flash_attn': 'on'})
        records.append(record(n_threads=4, flash_attn=0, samples_ts=[6.0]))
        self.assertEqual(L.select_best(records), {'threads': 4, 'flash_attn': 'off'})

    def test_missing_runtime_rejected(self):
        with self.assertRaises(ValueError):
            L.select_best([record(runtime='cpu')])


if __name__ == '__main__':
    unittest.main()
