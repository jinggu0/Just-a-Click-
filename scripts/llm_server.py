"""Run the pinned llama-server for local benchmarks. Loopback only; the key stays in the env."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request

from bench_env import hide_paths, process_memory

ROOT = Path(__file__).resolve().parents[1]
READY_TIMEOUT_SECONDS = 300
REQUEST_TIMEOUT_SECONDS = 600
# Loopback calls must ignore any proxy configured for the machine.
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def server_command(executable, model_path, port, context_tokens, threads,
                   flash_attn='off', batch=2048, ubatch=512, gpu_layers=99):
    """Loopback only; the API key travels in the environment, never on the command line."""
    return [str(executable), '-m', str(model_path), '--host', '127.0.0.1', '--port', str(port),
            '-c', str(context_tokens), '-np', '1', '-ngl', str(gpu_layers), '-t', str(threads),
            '--jinja', '--reasoning', 'off', '--flash-attn', flash_attn,
            '-b', str(batch), '-ub', str(ubatch)]


def open_url(request, timeout):
    return OPENER.open(request, timeout=timeout)


class LlamaServer:
    """One llama-server process, used as a context manager."""

    def __init__(self, executable, model_path, log_path, context_tokens=8192, threads=4, **options):
        self.executable, self.model_path, self.log_path = executable, model_path, log_path
        self.context_tokens, self.threads, self.options = context_tokens, threads, options
        self.port = self.key = self.process = self.log = self.startup_seconds = None

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *_):
        self.stop()

    def start(self):
        self.port, self.key = free_port(), secrets.token_hex(32)
        command = server_command(self.executable, self.model_path, self.port,
                                 self.context_tokens, self.threads, **self.options)
        self.log = self.log_path.open('w', encoding='utf-8')
        started = time.perf_counter()
        self.process = subprocess.Popen(command, stdout=self.log, stderr=self.log,
                                        env=dict(os.environ, LLAMA_API_KEY=self.key),
                                        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        self.wait_ready(started)
        return self

    def wait_ready(self, started, timeout=READY_TIMEOUT_SECONDS):
        while True:
            if self.process.poll() is not None:
                raise RuntimeError(f'llama-server exited {self.process.returncode}: {self.log_tail()}')
            if time.perf_counter() - started > timeout:
                self.stop()
                raise TimeoutError(f'llama-server not ready in {timeout}s: {self.log_tail()}')
            try:
                self.call('/health', None, 2)
            except (urllib.error.URLError, OSError, RuntimeError):
                time.sleep(0.5)
                continue
            self.startup_seconds = round(time.perf_counter() - started, 2)
            return self.startup_seconds

    def call(self, endpoint, payload, timeout):
        data = None if payload is None else json.dumps(payload, ensure_ascii=False).encode('utf-8')
        request = urllib.request.Request(f'http://127.0.0.1:{self.port}{endpoint}', data=data,
                                         headers={'Content-Type': 'application/json',
                                                  'Authorization': f'Bearer {self.key}'})
        try:
            with open_url(request, timeout) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            details = error.read(2048).decode('utf-8', errors='replace')
            raise RuntimeError(f'HTTP {error.code}: {hide_paths(details)}') from error

    def post(self, endpoint, payload, timeout=REQUEST_TIMEOUT_SECONDS):
        return self.call(endpoint, payload, timeout)

    def memory(self):
        handle = getattr(self.process, '_handle', None)
        return process_memory(handle) if handle else None

    def log_tail(self, lines=5):
        try:
            text = self.log_path.read_text('utf-8', errors='replace')
        except OSError:
            return ''
        return hide_paths('\n'.join(text.splitlines()[-lines:]))

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=30)
        if self.log:
            self.log.close()
            self.log = None
