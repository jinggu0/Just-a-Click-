import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location(
    'prepare_llm', Path(__file__).resolve().parents[1] / 'scripts/prepare_llm.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Response(io.BytesIO):
    def __init__(self, data, status=206, content_range='bytes 0-4/5'):
        super().__init__(data)
        self.status = status
        self.headers = {'Content-Range': content_range}


class DownloadTests(unittest.TestCase):
    def test_verified_range_activates_and_reuses_model(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'model.gguf'
            digest = hashlib.sha256(b'hello').hexdigest()
            with patch.object(MODULE.urllib.request, 'urlopen', return_value=Response(b'hello')) as fetch:
                MODULE.download_model('https://example.invalid/model', target, 5, digest)
                MODULE.download_model('https://example.invalid/model', target, 5, digest)
                self.assertEqual(fetch.call_count, 1)
                self.assertEqual(target.read_bytes(), b'hello')

    def test_wrong_digest_preserves_previous_model(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'model.gguf'
            target.write_bytes(b'previous')
            with patch.object(MODULE.urllib.request, 'urlopen', return_value=Response(b'hello')):
                with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'):
                    MODULE.download_model('https://example.invalid/model', target, 5, '0' * 64)
            self.assertEqual(target.read_bytes(), b'previous')

    def test_zip_extraction_rejects_escaping_member(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'bad.zip'
            with zipfile.ZipFile(archive, 'w') as zipped:
                zipped.writestr('../escape.txt', 'x')
            with self.assertRaisesRegex(RuntimeError, 'Unsafe archive path'):
                MODULE.safe_extract_zip(archive, Path(directory) / 'out')
            self.assertFalse((Path(directory) / 'escape.txt').exists())

    def test_zip_extraction_keeps_member_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'ok.zip'
            with zipfile.ZipFile(archive, 'w') as zipped:
                zipped.writestr('Release/tool.exe', 'bin')
            MODULE.safe_extract_zip(archive, Path(directory) / 'out')
            self.assertEqual((Path(directory) / 'out/Release/tool.exe').read_text(), 'bin')

    def test_ignored_range_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'model.gguf'
            with patch.object(MODULE.urllib.request, 'urlopen', return_value=Response(b'hello', status=200)):
                with self.assertRaisesRegex(RuntimeError, 'exact byte range'):
                    MODULE.download_model('https://example.invalid/model', target, 5, '0' * 64)
            self.assertFalse(target.exists())


if __name__ == '__main__':
    unittest.main()
