from pathlib import Path
import re
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import prepare_stt as P  # noqa: E402

SHA256 = re.compile(r'[0-9a-f]{64}')


class ConfigTests(unittest.TestCase):
    def test_pinned_files_have_sizes_and_hashes(self):
        runtime = P.load_config('stt-runtime.json')
        models = P.load_config('stt-models.json')
        data = P.load_config('stt-eval-data.json')
        items = runtime['assets'] + models['models'] + data['files']
        for item in items:
            self.assertGreater(item['size_bytes'], 0)
            self.assertRegex(item['sha256'], SHA256)
        ids = [model['id'] for model in models['models']]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertIn(models['screen_model'], ids)
        self.assertEqual({asset['build'] for asset in runtime['assets']}, {'cpu', 'blas'})
        self.assertEqual(data['license'], 'CC-BY-4.0')


class JobTests(unittest.TestCase):
    def test_runtime_jobs_extract_per_build(self):
        runtime = {'tag': 'b1', 'release_url': 'https://example.invalid/b1',
                   'assets': [{'build': 'cpu', 'filename': 'w.zip', 'size_bytes': 1, 'sha256': 'x'}]}
        url, archive, size, digest, destination = P.runtime_jobs(runtime)[0]
        self.assertEqual(url, 'https://example.invalid/b1/w.zip')
        self.assertEqual(archive, P.ROOT / 'downloads/whisper.cpp/b1/w.zip')
        self.assertEqual(destination, P.ROOT / 'runtimes/whisper-b1/cpu')

    def test_model_jobs_filter_by_id(self):
        models = {'repository': 'org/repo', 'revision': 'r1', 'models': [
            {'id': 'a', 'filename': 'a.bin', 'size_bytes': 1, 'sha256': 'x'},
            {'id': 'b', 'filename': 'b.bin', 'size_bytes': 2, 'sha256': 'y'}]}
        self.assertEqual(len(P.model_jobs(models)), 2)
        (url, target, size, digest), = P.model_jobs(models, ['b'])
        self.assertEqual(url, 'https://huggingface.co/org/repo/resolve/r1/b.bin')
        self.assertEqual((target, size), (P.ROOT / 'models/whisper/b.bin', 2))

    def test_data_jobs_keep_file_names(self):
        data = {'dataset': 'google/fleurs', 'config': 'ko_kr', 'revision': 'r2', 'files': [
            {'path': 'data/ko_kr/audio/test.tar.gz', 'size_bytes': 3, 'sha256': 'z'}]}
        (url, target, size, digest), = P.data_jobs(data)
        self.assertEqual(url, 'https://huggingface.co/datasets/google/fleurs/resolve/r2/'
                              'data/ko_kr/audio/test.tar.gz')
        self.assertEqual(target, P.ROOT / 'downloads/fleurs/ko_kr/test.tar.gz')

    def test_large_files_use_resumable_ranges(self):
        self.assertIs(P.fetcher_for(65 * 1024 * 1024), P.download_model)
        self.assertIs(P.fetcher_for(1024), P.download)


if __name__ == '__main__':
    unittest.main()
