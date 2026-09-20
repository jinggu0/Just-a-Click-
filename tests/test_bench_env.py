from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import bench_env as E  # noqa: E402


class EnvironmentTests(unittest.TestCase):
    def test_detects_competing_inference_process(self):
        csv_text = ('"System","4","Services","0","132 K"\n'
                    '"llama-server.exe","1234","Console","1","5,000 K"\n'
                    '"whisper-cli.exe","99","Console","1","9 K"\n')
        blocked = {'llama-server.exe', 'whisper-cli.exe'}
        self.assertEqual(E.running_processes(csv_text, blocked),
                         ['llama-server.exe', 'whisper-cli.exe'])
        self.assertEqual(E.running_processes('"python.exe","1","Console","1","1 K"\n', blocked), [])

    def test_power_description(self):
        self.assertEqual(E.describe_power(1, 80, 0),
                         {'ac_power': True, 'battery_percent': 80, 'battery_saver': False})
        self.assertEqual(E.describe_power(0, 255, 1),
                         {'ac_power': False, 'battery_percent': None, 'battery_saver': True})

    def test_power_mode_names_known_overlays(self):
        mode = E.describe_power_mode('DED574B5-45A0-4F42-8737-46345C09C238',
                                     '961cc777-2547-4f9d-8174-7d86181b8a7a')
        self.assertEqual((mode['ac_mode'], mode['dc_mode']),
                         ('best_performance', 'best_power_efficiency'))
        other = E.describe_power_mode('00000000-0000-0000-0000-000000000000', None)
        self.assertEqual((other['ac_mode'], other['dc_mode']), ('unknown', None))

    def test_keep_awake_blocks_sleep_and_display_off(self):
        with patch.object(E.ctypes.windll.kernel32, 'SetThreadExecutionState') as api:
            with E.keep_awake():
                api.assert_called_once_with(0x80000003)
            api.assert_called_with(0x80000000)

    def test_describe_memory_reports_mebibytes(self):
        class Counters:
            WorkingSetSize, PeakWorkingSetSize = 100 * 1024 * 1024, 150 * 1024 * 1024
            PagefileUsage, PeakPagefileUsage = 200 * 1024 * 1024, 250 * 1024 * 1024
        self.assertEqual(E.describe_memory(Counters()),
                         {'working_set_mib': 100.0, 'peak_working_set_mib': 150.0,
                          'private_mib': 200.0, 'peak_private_mib': 250.0})

    def test_process_memory_reads_this_process(self):
        memory = E.process_memory(E.ctypes.windll.kernel32.GetCurrentProcess())
        self.assertGreater(memory['private_mib'], 0)
        self.assertGreaterEqual(memory['peak_working_set_mib'], memory['working_set_mib'])

    def test_hide_paths_masks_checkout_and_home(self):
        text = f'{E.ROOT}\\models\\a.bin {Path.home().as_posix()}/x.wav'
        self.assertEqual(E.hide_paths(text), '<repo>\\models\\a.bin <home>/x.wav')

    def test_unique_path_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'r.json'
            path.write_text('{}')
            self.assertEqual(E.unique_path(path).name, 'r-2.json')


if __name__ == '__main__':
    unittest.main()
