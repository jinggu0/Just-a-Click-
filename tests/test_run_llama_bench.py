from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import run_llama_bench as R  # noqa: E402

LINE = ('{"build_commit": "0a8b29a60", "build_number": 10994, "backends": "Vulkan", '
        '"cpu_info": "cpu", "gpu_info": "gpu", '
        '"model_filename": "C:/m/Qwen3-8B-Q5_K_M.gguf", "model_type": "qwen3", "n_batch": 2048, '
        '"n_ubatch": 512, "n_threads": 8, "n_gpu_layers": 99, "flash_attn": 1, "n_prompt": 0, '
        '"n_gen": 128, "n_depth": 0, "test_time": "t", "samples_ts": [4.0]}')


class JobTests(unittest.TestCase):
    def test_screen_covers_cpu_and_vulkan_grid(self):
        jobs = R.screen_jobs()
        self.assertEqual([j['name'] for j in jobs], [
            'cpu-t4', 'cpu-t8', 'vulkan-t4-fa-off', 'vulkan-t4-fa-on',
            'vulkan-t8-fa-off', 'vulkan-t8-fa-on'])
        vulkan = jobs[2]['args']
        self.assertEqual(vulkan[vulkan.index('-d') + 1], '0,2048')
        self.assertEqual(jobs[0]['args'][jobs[0]['args'].index('-ngl') + 1], '0')

    def test_depth_jobs_use_selection_and_isolate_risky_batch(self):
        jobs = R.depth_jobs({'threads': 4, 'flash_attn': 'on'})
        self.assertEqual([j['allow_failure'] for j in jobs], [False, True])
        for job in jobs:
            self.assertEqual(job['args'][job['args'].index('-t') + 1], '4')
            self.assertEqual(job['args'][job['args'].index('-fa') + 1], 'on')
        self.assertEqual(jobs[1]['args'][-4:], ['-ub', '512', '-d', '8192'])

    def test_command_uses_pinned_runtime_and_jsonl(self):
        job = R.screen_jobs()[2]
        cmd = R.command(job, Path('m.gguf'), Path('runtimes/b10994'), 3)
        self.assertEqual(Path(cmd[0]), Path('runtimes/b10994/vulkan/llama-bench.exe'))
        self.assertEqual(cmd[1:8], ['-m', 'm.gguf', '-r', '3', '-o', 'jsonl', '--progress'])

    def test_competing_speech_processes_also_block(self):
        self.assertIn('whisper-cli.exe', R.BLOCKING_PROCESSES)

    def test_overall_status_distinguishes_expected_failures(self):
        ok = {'status': 'completed', 'allow_failure': False}
        risky = {'status': 'failed', 'allow_failure': True}
        broken = {'status': 'timeout', 'allow_failure': False}
        self.assertEqual(R.overall_status([ok]), 'completed')
        self.assertEqual(R.overall_status([ok, risky]), 'completed_with_expected_failures')
        self.assertEqual(R.overall_status([ok, risky, broken]), 'failed')


class RunJobTests(unittest.TestCase):
    def setUp(self):
        self.job = {'name': 'vulkan-depth-ub512', 'runtime': 'vulkan',
                    'allow_failure': True, 'args': ['-d', '8192']}

    def test_crash_keeps_completed_records(self):
        crashed = subprocess.CompletedProcess(['x'], 3, stdout=LINE + '\n{"n_prompt": 51')
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(R.subprocess, 'run', return_value=crashed):
            result, records = R.run_job(self.job, ['x'], Path(directory), 10)
            self.assertTrue((Path(directory) / 'vulkan-depth-ub512.jsonl').exists())
        self.assertEqual((result['status'], result['returncode']), ('failed', 3))
        self.assertEqual((result['records'], result['malformed_lines']), (1, 1))
        self.assertEqual(records[0]['runtime'], 'vulkan')
        self.assertEqual(result['error_tail'], '')

    def test_error_tail_hides_checkout_path(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'job.log'
            log.write_text(f'loaded {R.ROOT}\\runtimes\nggml_vulkan: device lost\n', 'utf-8')
            self.assertEqual(R.tail(log), 'loaded <repo>\\runtimes\nggml_vulkan: device lost')

    def test_timeout_is_recorded_not_raised(self):
        expired = subprocess.TimeoutExpired(['x'], 10, output=LINE.encode('utf-8'))
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(R.subprocess, 'run', side_effect=expired):
            result, records = R.run_job(self.job, ['x'], Path(directory), 10)
        self.assertEqual((result['status'], result['returncode'], len(records)),
                         ('timeout', None, 1))


if __name__ == '__main__':
    unittest.main()
