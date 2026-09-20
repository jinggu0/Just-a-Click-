import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import llm_server as L  # noqa: E402


def response(payload):
    return io.BytesIO(json.dumps(payload).encode('utf-8'))


class FakeProcess:
    def __init__(self, returncode=None):
        self.returncode = returncode
        self._handle, self.terminated = 7, False

    def poll(self):
        return self.returncode

    def terminate(self):
        self.terminated, self.returncode = True, 0

    def wait(self, timeout=None):
        return self.returncode


class CommandTests(unittest.TestCase):
    def test_server_command_stays_on_loopback_without_the_key(self):
        command = L.server_command(Path('llama-server.exe'), Path('m.gguf'), 8080, 8192, 2)
        self.assertEqual(command[3:7], ['--host', '127.0.0.1', '--port', '8080'])
        for flag, value in (('-c', '8192'), ('-t', '2'), ('-ngl', '99'), ('--flash-attn', 'off')):
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertNotIn('--api-key', command)

    def test_free_port_is_usable(self):
        self.assertGreater(L.free_port(), 1024)


class StartTests(unittest.TestCase):
    def server(self, directory, process):
        return L.LlamaServer(Path('llama-server.exe'), Path('m.gguf'),
                             Path(directory) / 'server.log', context_tokens=4096, threads=2)

    def test_start_passes_the_key_through_the_environment(self):
        seen = {}

        def popen(command, **options):
            seen.update(command=command, env=options['env'])
            return FakeProcess()

        with tempfile.TemporaryDirectory() as directory:
            server = self.server(directory, None)
            with patch.object(L.subprocess, 'Popen', side_effect=popen), \
                    patch.object(L, 'open_url', return_value=response({'status': 'ok'})):
                server.start()
            self.assertEqual(seen['env']['LLAMA_API_KEY'], server.key)
            self.assertNotIn(server.key, ' '.join(seen['command']))
            self.assertIsNotNone(server.startup_seconds)
            server.stop()

    def test_wait_ready_reports_a_server_that_exited(self):
        with tempfile.TemporaryDirectory() as directory:
            server = self.server(directory, None)
            server.log_path.write_text(f'load failed {L.ROOT}\\models\\m.gguf\n', 'utf-8')
            server.process = FakeProcess(returncode=3)
            with self.assertRaises(RuntimeError) as caught:
                server.wait_ready(started=0.0)
        self.assertIn('<repo>', str(caught.exception))
        self.assertNotIn(str(L.ROOT), str(caught.exception))


class RequestTests(unittest.TestCase):
    def server(self):
        server = L.LlamaServer(Path('x'), Path('m.gguf'), Path('server.log'))
        server.port, server.key = 9999, 'secret-token'
        return server

    def test_post_sends_authorised_json(self):
        seen = {}

        def open_url(request, timeout):
            seen.update(url=request.full_url, auth=request.get_header('Authorization'),
                        body=json.loads(request.data.decode('utf-8')), timeout=timeout)
            return response({'choices': [{'message': {'content': '초안'}}]})

        with patch.object(L, 'open_url', side_effect=open_url):
            result = self.server().post('/v1/chat/completions', {'messages': [], '한글': True}, 30)
        self.assertEqual(seen['url'], 'http://127.0.0.1:9999/v1/chat/completions')
        self.assertEqual(seen['auth'], 'Bearer secret-token')
        self.assertEqual((seen['body']['한글'], seen['timeout']), (True, 30))
        self.assertEqual(result['choices'][0]['message']['content'], '초안')

    def test_http_error_hides_paths(self):
        error = urllib.error.HTTPError('http://127.0.0.1:9999/x', 500, 'boom', {},
                                       io.BytesIO(f'at {L.ROOT}\\models'.encode('utf-8')))
        with patch.object(L, 'open_url', side_effect=error):
            with self.assertRaisesRegex(RuntimeError, 'HTTP 500') as caught:
                self.server().post('/x', {}, 5)
        self.assertIn('<repo>', str(caught.exception))
        self.assertNotIn(str(L.ROOT), str(caught.exception))


if __name__ == '__main__':
    unittest.main()
