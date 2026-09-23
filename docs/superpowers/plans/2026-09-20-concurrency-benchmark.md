# 녹음 중 STT·LLM 동시 실행 측정 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 실제 강의 녹음처럼 음성 조각을 실시간 속도로 흘려보내며 whisper.cpp 전사와 llama.cpp 구간 초안을 함께 실행해, 녹음 중 동시 처리 가능 여부와 RAM 16GB 여유, 발열·배터리 영향을 추정한다.

**Architecture:** 실행기가 조각 도착을 실시간으로 재현하고, 전사 작업자와 초안 작업자가 각각 스레드로 돌며 상주 llama-server에 요청한다. 판정 계산은 순수 함수 모듈로 분리하고, 서버 수명 관리와 성능 카운터 수집도 각각 독립 모듈로 둔다.

**Tech Stack:** Python 3.11 표준 라이브러리(`threading`, `queue`, `subprocess`, `urllib`, `ctypes`, `unittest`), Windows PowerShell 성능 카운터, whisper.cpp `b5130` OpenBLAS 빌드, llama.cpp `b10994` Vulkan `llama-server`, Qwen3-8B Q5_K_M, Whisper large-v3-turbo f16.

## Global Constraints

- 설계 문서: `docs/superpowers/specs/2026-09-19-concurrency-benchmark-design.md`. 모든 판정은 추정이며 앱 지연시간·출시 품질 판정이 아니다.
- Python 3.11 이상 표준 라이브러리만 쓴다. 새 의존성을 추가하지 않는다. 전체 테스트 명령은 `python -m unittest discover -s tests -v`다.
- 판정 조건은 AC 전원·Windows 전원 모드 "최고 성능"이다. 전원 설정·드라이버는 읽기만 하고 사용자가 바꾼다. 측정 중에는 절전과 화면 꺼짐만 요청한다.
- 시간 기준: 전사 지연 95번째 백분위 ≤ 30초·최대 ≤ 60초, 60분 이상 측정에서 지연 추세 ≤ 10초, 구간 초안 최대 ≤ 150초, 녹음 종료 후 5분 재추정 ≤ 300초.
- 메모리 기준(16GB 추정): GPU 공유 메모리 ≤ 7,838MiB(물리 메모리의 절반), 합계 ≤ 11,981MiB(OS·다른 앱 4GiB 가정). 겹쳐 잡히는 몫은 한 번만 센다.
- llama-server는 127.0.0.1에만 열고 임의 포트와 임의 토큰을 쓴다. 토큰은 환경 변수로만 넘기고 명령줄·로그·요약에 남기지 않는다. `whisper-server`는 쓰지 않는다.
- 모델·빌드·조각 WAV·원출력·로그는 `models/`, `runtimes/`, `artifacts/`에만 둔다. 커밋하는 요약에는 전사·초안 전문과 로컬 경로를 남기지 않는다. 결과 파일은 덮어쓰지 않는다.
- 문서는 한국어, 코드·주석은 영어로 쓴다. 각 Task는 검증 후 해당 경로만 stage하여 main에 커밋하고, 메시지 끝에 `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`를 붙인다. push하지 않는다.

## 사전 확인 사실 (2026-09-20, 계획 작성 중 직접 확인)

- `llama-server`(b10994 Vulkan)는 6.7초 만에 준비됐고, `/v1/chat/completions` 응답에 `timings`(`prompt_n`, `prompt_ms`, `prompt_per_second`, `predicted_n`, `predicted_ms`, `predicted_per_second`)가 들어 있다. `ignore_eos: true`와 `max_tokens: 32`를 주면 정확히 32토큰을 생성하고 `finish_reason`은 `length`다. 따라서 창마다 같은 출력량(300토큰)으로 고정할 수 있다.
- 같은 서버(컨텍스트 4,096)의 프로세스 전용 메모리는 6,656MiB, 프로세스별 GPU 공유 메모리는 6,494MiB였다. 두 값이 거의 같으므로 Vulkan 할당이 양쪽에 함께 잡힌다. 16GB 추정에서는 큰 값만 한 번 센다.
- `\GPU Process Memory(pid_<PID>*)\Shared Usage`와 `\GPU Adapter Memory(*)\Shared Usage` 카운터, `root\wmi`의 `BatteryStatus`(`DischargeRate` mW, `RemainingCapacity` mWh)와 `BatteryFullChargedCapacity`를 이 기기에서 읽을 수 있다.
- 종료된 자식 프로세스의 최대 메모리는 Popen 핸들로 `GetProcessMemoryInfo`를 호출해 읽을 수 있다(확인: 300MB를 쓰는 자식의 최대 작업 집합 296MiB).
- 성능 카운터 수집 스크립트를 12초 돌려 20종(CPU, 온도, 열 제한, 전력 4종, 가용·커밋 메모리, GPU 어댑터 공유 메모리 3종, 프로세스 CPU, 배터리 4종)을 기록하는 것을 확인했다.
- 이 계획의 코드는 저장소 사본에서 먼저 구현했고 기존 테스트를 포함해 113개가 통과했다.
- 선행 측정값: STT는 large-v3-turbo f16·OpenBLAS·8스레드에서 조각 RTF 중앙값 0.422([STT 보고서](../../validation/2026-09-18-stt-bench.md)), LLM은 Vulkan·스레드 4·FA off에서 5분 창 초안 24.5~26.3초 추정([LLM 재측정](../../validation/2026-09-17-llama-bench-best-performance.md)).

## File Structure

| 경로 | 구분 | 책임 |
| --- | --- | --- |
| `scripts/bench_env.py` | 수정 | 프로세스 메모리 조회(`describe_memory`, `process_memory`) 추가 |
| `scripts/run_stt_bench.py` | 수정 | `run_chunk`가 Popen으로 실행하고 조각별 최대 메모리를 기록 |
| `scripts/llm_server.py` | 생성 | llama-server 시작·준비 대기·인증 요청·메모리·종료 |
| `scripts/sample_counters.ps1` | 생성 | 5초 간격 성능 카운터·배터리 기록, 감시 프로세스가 끝나면 종료 |
| `scripts/concurrency_metrics.py` | 생성 | 창 경계, 전사 추종, 초안 통계, 종료 후 추정, 메모리 예산, 배터리 소모, 배분 선택 |
| `scripts/run_concurrency.py` | 생성 | `probe`·`screen`·`run` 단계, 실시간 도착 스케줄러, 전사·초안 작업자, 요약·표 작성 |
| `tests/test_bench_env.py`, `tests/test_run_stt_bench.py` | 수정 | 메모리 조회와 `run_chunk` 변경 |
| `tests/test_llm_server.py`, `tests/test_concurrency_metrics.py`, `tests/test_run_concurrency.py` | 생성 | 새 모듈 테스트 |
| `evaluation/README.md` | 수정 | 동시 실행 측정 사용법 |
| `evaluation/results/<날짜>-concurrency-<단계>-<조건>.json` | 측정 후 생성 | 요약 |
| `docs/validation/<날짜>-concurrency.md`, `docs/decisions/0009-concurrent-processing.md` | 측정 후 생성 | 보고서, 결정 초안 |

`<날짜>`는 측정을 실행한 날짜이며, `<조건>`은 실행기가 전원 상태·모드에서 자동으로 붙이는 이름이다(예: `ac-best_performance`).

---

### Task 1: 프로세스 메모리 조회

**Files:**
- Modify: `scripts/bench_env.py`
- Test: `tests/test_bench_env.py`

**Interfaces:**
- Consumes: 없음.
- Produces: `MEBIBYTE`, `describe_memory(counters) -> dict`(키 `working_set_mib`, `peak_working_set_mib`, `private_mib`, `peak_private_mib`), `process_memory(handle) -> dict | None`.

- [ ] **Step 1: 실패하는 테스트 추가**

`tests/test_bench_env.py`의 `test_hide_paths_masks_checkout_and_home` 앞에 다음 두 테스트를 추가한다.

```python
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
```

Run: `python -m unittest discover -s tests -p test_bench_env.py`
Expected: FAIL — `AttributeError: module 'bench_env' has no attribute 'describe_memory'`

- [ ] **Step 2: 메모리 조회 구현**

`scripts/bench_env.py`의 import 부분에서 `import ctypes` 다음 줄에 `from ctypes import wintypes`를 추가하고, `ROOT = Path(__file__).resolve().parents[1]` 다음 줄에 `MEBIBYTE = 1024 * 1024`를 추가한다. 그리고 `@contextmanager` 바로 앞에 다음을 넣는다.

```python
class _MemoryCounters(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD),
                ('PeakWorkingSetSize', ctypes.c_size_t), ('WorkingSetSize', ctypes.c_size_t),
                ('QuotaPeakPagedPoolUsage', ctypes.c_size_t),
                ('QuotaPagedPoolUsage', ctypes.c_size_t),
                ('QuotaPeakNonPagedPoolUsage', ctypes.c_size_t),
                ('QuotaNonPagedPoolUsage', ctypes.c_size_t),
                ('PagefileUsage', ctypes.c_size_t), ('PeakPagefileUsage', ctypes.c_size_t)]


def describe_memory(counters):
    """Working set is resident memory; private (pagefile usage) is what the process commits."""
    return {'working_set_mib': round(counters.WorkingSetSize / MEBIBYTE, 1),
            'peak_working_set_mib': round(counters.PeakWorkingSetSize / MEBIBYTE, 1),
            'private_mib': round(counters.PagefileUsage / MEBIBYTE, 1),
            'peak_private_mib': round(counters.PeakPagefileUsage / MEBIBYTE, 1)}


def process_memory(handle):
    """Memory of a process we started; still readable after it exits while the handle is open."""
    counters = _MemoryCounters()
    counters.cb = ctypes.sizeof(counters)
    if not ctypes.windll.psapi.GetProcessMemoryInfo(wintypes.HANDLE(int(handle)),
                                                    ctypes.byref(counters), counters.cb):
        return None
    return describe_memory(counters)
```

Run: `python -m unittest discover -s tests -p test_bench_env.py`
Expected: PASS — `Ran 8 tests`, `OK`

- [ ] **Step 3: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 84 tests`, `OK`

```bash
git add scripts/bench_env.py tests/test_bench_env.py
git commit -m "feat: read process memory of benchmark child processes" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: 조각별 최대 메모리 기록

**Files:**
- Modify: `scripts/run_stt_bench.py`
- Test: `tests/test_run_stt_bench.py`

**Interfaces:**
- Consumes: `bench_env.process_memory`.
- Produces: `peak_memory(process) -> dict`, `run_chunk`의 기록에 `peak_working_set_mib`·`peak_private_mib` 추가(시간 초과 기록에도 포함).

- [ ] **Step 1: 테스트를 Popen 방식으로 바꾼다**

`tests/test_run_stt_bench.py`의 `class RunChunkTests(unittest.TestCase):` 블록 전체(다음 `class SummaryTests` 앞까지)를 다음으로 바꾼다.

```python
class FakeProcess:
    """Stands in for Popen: communicate() returns stderr or times out once."""

    def __init__(self, returncode=0, stderr='', timeout=False):
        self.returncode, self.stderr, self.timeout = returncode, stderr, timeout
        self._handle, self.killed = 7, False

    def communicate(self, timeout=None):
        if self.timeout and not self.killed:
            raise subprocess.TimeoutExpired(['x'], timeout or 1)
        return '', self.stderr

    def kill(self):
        self.killed = True


class RunChunkTests(unittest.TestCase):
    def chunk_result(self, process, prefix):
        with patch.object(S.subprocess, 'Popen', return_value=process), \
                patch.object(S, 'process_memory', return_value={
                    'working_set_mib': 1.0, 'peak_working_set_mib': 600.0,
                    'private_mib': 2.0, 'peak_private_mib': 512.0}):
            return S.run_chunk(['x'], prefix, CHUNK)

    def test_completed_chunk_measures_processing_and_memory(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / 'fleurs-ko-001'
            Path(f'{prefix}.json').write_text(
                json.dumps({'transcription': [{'text': ' 다리미 간격'}]}), 'utf-8')
            result = self.chunk_result(FakeProcess(stderr=STDERR), prefix)
        self.assertEqual(result['status'], 'completed')
        self.assertEqual((result['load_seconds'], result['processing_seconds']), (0.5, 6.0))
        self.assertEqual(result['rtf'], 0.3)
        self.assertEqual((result['errors'], result['ref_chars'], result['fallbacks']), (1, 5, 1))
        self.assertEqual(result['decoding']['beams'], 5)
        self.assertEqual((result['peak_private_mib'], result['peak_working_set_mib']), (512.0, 600.0))
        self.assertFalse(result['anomaly_suspect'])

    def test_failed_exit_hides_paths(self):
        process = FakeProcess(returncode=3, stderr=f'error in {S.ROOT}\\m.bin')
        result = self.chunk_result(process, Path('missing'))
        self.assertEqual((result['status'], result['returncode']), ('failed', 3))
        self.assertIn('<repo>\\m.bin', result['error'])
        self.assertNotIn(str(S.ROOT), result['error'])

    def test_missing_output_or_timeout(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.chunk_result(FakeProcess(stderr=STDERR), Path(directory) / 'none')
        self.assertEqual(result['status'], 'failed')
        process = FakeProcess(timeout=True)
        timed_out = self.chunk_result(process, Path('p'))
        self.assertEqual((timed_out['status'], timed_out['peak_private_mib']), ('timeout', 512.0))
        self.assertTrue(process.killed)
```

Run: `python -m unittest discover -s tests -p test_run_stt_bench.py`
Expected: FAIL — `KeyError: 'peak_private_mib'`(기록에 메모리가 아직 없다)

- [ ] **Step 2: `run_chunk`를 Popen으로 바꾸고 메모리를 기록한다**

`scripts/run_stt_bench.py`의 import에서 `bench_env`에 `process_memory`를 추가한다.

```python
from bench_env import (competing_processes, hide_paths, keep_awake, power_mode, power_status,
                       process_memory, unique_path, utc_now)
```

`def run_chunk(` 부터 `def summarize(` 직전까지를 다음으로 바꾼다.

```python
def peak_memory(process):
    """Peak memory of the finished process; empty when the handle cannot be read."""
    handle = getattr(process, '_handle', None)  # Popen keeps the Windows handle open until closed.
    memory = (process_memory(handle) or {}) if handle else {}
    return {key: memory[key] for key in ('peak_working_set_mib', 'peak_private_mib') if key in memory}


def run_chunk(cmd, output_prefix, chunk, timeout=CHUNK_TIMEOUT_SECONDS):
    started = time.perf_counter()
    record = {'chunk_id': chunk['chunk_id'], 'seconds': chunk['seconds']}
    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                               encoding='utf-8', errors='replace',
                               creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    try:
        stderr = process.communicate(timeout=timeout)[1]
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        return dict(record, status='timeout', wall_seconds=round(time.perf_counter() - started, 2),
                    **peak_memory(process))
    record.update(returncode=process.returncode,
                  wall_seconds=round(time.perf_counter() - started, 2), **peak_memory(process))
    try:
        if process.returncode != 0:
            raise RuntimeError(f'whisper-cli exited with {process.returncode}')
        timings = parse_timings(stderr)
        text = transcript_text(json.loads(Path(f'{output_prefix}.json').read_text('utf-8')))
    except (RuntimeError, ValueError, KeyError, OSError) as error:
        tail = '\n'.join(stderr.splitlines()[-3:])
        return dict(record, status='failed', error=hide_paths(f'{error}\n{tail}'.strip()))
    processing = (timings['total_ms'] - timings['load_ms']) / 1000
    errors = char_errors(chunk['reference'], text)
    return dict(record, status='completed', load_seconds=round(timings['load_ms'] / 1000, 3),
                processing_seconds=round(processing, 3),
                rtf=round(processing / chunk['seconds'], 4),
                fallbacks=timings.get('fallbacks'), decoding=timings.get('decoding'),
                anomaly_suspect=suspected_anomaly(errors), **errors)
```

Run: `python -m unittest discover -s tests -p test_run_stt_bench.py`
Expected: PASS — `Ran 13 tests`, `OK`

- [ ] **Step 3: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 84 tests`, `OK`

```bash
git add scripts/run_stt_bench.py tests/test_run_stt_bench.py
git commit -m "feat: record peak memory for each transcribed chunk" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: llama-server 수명 관리

**Files:**
- Create: `scripts/llm_server.py`
- Test: `tests/test_llm_server.py`

**Interfaces:**
- Consumes: `bench_env.hide_paths`, `bench_env.process_memory`.
- Produces:
  - `free_port() -> int`, `server_command(executable, model_path, port, context_tokens, threads, flash_attn='off', batch=2048, ubatch=512, gpu_layers=99) -> list[str]`, `open_url(request, timeout)`
  - `LlamaServer(executable, model_path, log_path, context_tokens=8192, threads=4, **options)`: 컨텍스트 관리자. `start()`, `wait_ready(started, timeout=300)`, `call(endpoint, payload, timeout)`, `post(endpoint, payload, timeout=600)`, `memory()`, `log_tail(lines=5)`, `stop()`. 속성 `port`, `key`, `process`, `startup_seconds`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_llm_server.py`:

```python
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
```

Run: `python -m unittest discover -s tests -p test_llm_server.py`
Expected: FAIL — `ModuleNotFoundError: No module named 'llm_server'`

- [ ] **Step 2: 모듈 구현**

`scripts/llm_server.py`:

```python
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
```

Run: `python -m unittest discover -s tests -p test_llm_server.py`
Expected: PASS — `Ran 6 tests`, `OK`

- [ ] **Step 3: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 90 tests`, `OK`

```bash
git add scripts/llm_server.py tests/test_llm_server.py
git commit -m "feat: manage the local llama-server for benchmarks" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: 성능 카운터 기록 스크립트

**Files:**
- Create: `scripts/sample_counters.ps1`

**Interfaces:**
- Consumes: 없음.
- Produces: `scripts/sample_counters.ps1 -Csv <경로> -WatchPid <PID> [-IntervalSeconds 5]`. `timestamp,counter,value` 형식으로 덧붙이며, 감시 대상 프로세스가 사라지면 끝난다.

- [ ] **Step 1: 스크립트 작성**

`scripts/sample_counters.ps1`:

```powershell
# Append CPU, thermal, power, memory, GPU and battery samples until the watched process exits.
# Read-only: it changes no system setting.
param(
  [Parameter(Mandatory = $true)][string]$Csv,
  [Parameter(Mandatory = $true)][int]$WatchPid,
  [int]$IntervalSeconds = 5
)

$counters = @(
  '\Processor(_Total)\% Processor Time',
  '\Processor Information(_Total)\% Processor Performance',
  '\Thermal Zone Information(*)\Temperature',
  '\Thermal Zone Information(*)\% Passive Limit',
  '\Energy Meter(*)\Power',
  '\Memory\Available MBytes',
  '\Memory\Committed Bytes',
  '\GPU Adapter Memory(*)\Shared Usage',
  '\Process(whisper-cli*)\% Processor Time',
  '\Process(llama-server*)\% Processor Time',
  '\Process(explorer*)\% Processor Time',
  '\Process(searchindexer*)\% Processor Time'
)

if (-not (Test-Path $Csv)) { 'timestamp,counter,value' | Out-File -Encoding utf8 $Csv }

while (Get-Process -Id $WatchPid -ErrorAction SilentlyContinue) {
  $rows = New-Object System.Collections.Generic.List[string]
  $sample = Get-Counter -Counter $counters -ErrorAction SilentlyContinue
  if ($sample) {
    foreach ($item in $sample.CounterSamples) {
      $rows.Add(('{0},{1},{2}' -f $sample.Timestamp.ToString('o'),
                 ($item.Path -replace '^\\\\[^\\]+', ''), $item.CookedValue))
    }
  }
  $stamp = (Get-Date).ToString('o')
  foreach ($battery in (Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue)) {
    $rows.Add(('{0},\battery\discharge rate,{1}' -f $stamp, $battery.DischargeRate))
    $rows.Add(('{0},\battery\charge rate,{1}' -f $stamp, $battery.ChargeRate))
    $rows.Add(('{0},\battery\remaining capacity,{1}' -f $stamp, $battery.RemainingCapacity))
    $rows.Add(('{0},\battery\power online,{1}' -f $stamp, [int][bool]$battery.PowerOnline))
  }
  if ($rows.Count -gt 0) { $rows | Out-File -Append -Encoding utf8 $Csv }
  Start-Sleep -Seconds $IntervalSeconds
}
```

- [ ] **Step 2: 12초 동안 실제로 기록되는지 확인**

Run:

```bash
python - <<'EOF'
import subprocess, sys
from pathlib import Path
csv = Path('artifacts/sampler-check.csv')
watched = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(12)'])
sampler = subprocess.Popen(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                            'scripts/sample_counters.ps1', '-Csv', str(csv),
                            '-WatchPid', str(watched.pid), '-IntervalSeconds', '3'])
watched.wait()
sampler.wait(timeout=60)
lines = csv.read_text('utf-8-sig').splitlines()
names = sorted({line.split(',')[1] for line in lines[1:]})
print('rows', len(lines), 'counters', len(names))
print('\n'.join(names))
EOF
```

Expected: `rows` 40 이상, `counters` 15 이상이며 목록에 `\processor(_total)\% processor time`, `\thermal zone information(\_tz.tz00)\temperature`, `\memory\available mbytes`, `\gpu adapter memory(...)\shared usage`, `\battery\remaining capacity`가 있다. 배터리 항목은 배터리가 있는 기기에서만 나온다.

- [ ] **Step 3: 커밋**

```bash
git add scripts/sample_counters.ps1
git commit -m "feat: sample CPU, thermal, memory, GPU and battery counters" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: 동시 실행 판정 지표

**Files:**
- Create: `scripts/concurrency_metrics.py`
- Test: `tests/test_concurrency_metrics.py`

**Interfaces:**
- Consumes: `stt_metrics.percentile`.
- Produces:
  - 상수 `KEEPUP_P95_SECONDS`(30.0), `KEEPUP_MAX_SECONDS`(60.0), `DRIFT_LIMIT_SECONDS`(10.0), `WINDOW_SECONDS`(300.0), `WINDOW_MAX_SECONDS`(150.0), `FINAL_INTEGRATION_SECONDS`(40.3), `SHARED_GPU_LIMIT_MIB`, `APP_BUDGET_MIB`
  - `closed_window(pending, window_seconds=300.0) -> list | None`
  - `keepup(chunks, minutes) -> dict`(키 `chunks`, `completed`, `failures`, `lag_median`, `lag_p95`, `lag_max`, `queued_seconds_max`, `rtf_median`, `lag_drift`, `ok`)
  - `window_stats(windows) -> dict`(키 `windows`, `completed`, `failures`, `seconds_median`, `seconds_p95`, `seconds_max`, `wait_seconds_max`, `input_tokens_median`, `prompt_tps_median`, `generate_tps_median`, `duty_median`, `within_limit`)
  - `post_recording_estimate(window_seconds_max) -> dict`, `memory_budget(stt_private_mib, llm_private_mib, gpu_shared_mib, double_counted=False) -> dict`
  - `battery_drain(samples, full_mwh) -> dict | None`, `select_allocation(entries) -> dict | None`, `suspended(sample_seconds, gap_seconds=60) -> bool`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_concurrency_metrics.py`:

```python
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
```

Run: `python -m unittest discover -s tests -p test_concurrency_metrics.py`
Expected: FAIL — `ModuleNotFoundError: No module named 'concurrency_metrics'`

- [ ] **Step 2: 모듈 구현**

`scripts/concurrency_metrics.py`:

```python
"""Judgement helpers for the concurrent STT and LLM benchmark. Estimates, not app latency."""
import statistics

from stt_metrics import percentile

KEEPUP_P95_SECONDS, KEEPUP_MAX_SECONDS = 30.0, 60.0
DRIFT_LIMIT_SECONDS, DRIFT_MIN_MINUTES, DRIFT_SPAN_SECONDS = 10.0, 60, 1800
WINDOW_SECONDS, WINDOW_MAX_SECONDS = 300.0, 150.0
BACKLOG_WINDOWS, FINAL_INTEGRATION_SECONDS = 2, 40.3
POST_RECORDING_TARGET_SECONDS = 300.0
TARGET_RAM_MIB = 15.7 * 1024  # a "16 GB" laptop reports about 15.7 GiB usable
SHARED_GPU_LIMIT_MIB = TARGET_RAM_MIB / 2  # Windows caps shared GPU memory at half of RAM
OS_RESERVE_MIB = 4 * 1024
APP_BUDGET_MIB = TARGET_RAM_MIB - OS_RESERVE_MIB
ALLOCATION_TIE = 0.10
SUSPEND_GAP_SECONDS = 60


def closed_window(pending, window_seconds=WINDOW_SECONDS):
    """Leading chunks whose audio reaches the window length, or None while it is short."""
    total = 0.0
    for index, chunk in enumerate(pending, 1):
        total += chunk['seconds']
        if total >= window_seconds:
            return pending[:index]
    return None


def keepup(chunks, minutes):
    """Transcription lag runs from the end of a chunk's audio to the end of its transcription."""
    done = [c for c in chunks if c['status'] == 'completed']
    result = {'chunks': len(chunks), 'completed': len(done), 'failures': len(chunks) - len(done)}
    lags = [c['lag_seconds'] for c in done]
    if not lags:
        return dict(result, ok=False)
    drift = None
    if minutes >= DRIFT_MIN_MINUTES:
        early = [c['lag_seconds'] for c in done if c['arrival_seconds'] <= DRIFT_SPAN_SECONDS]
        late = [c['lag_seconds'] for c in done
                if c['arrival_seconds'] >= minutes * 60 - DRIFT_SPAN_SECONDS]
        if early and late:
            drift = round(statistics.median(late) - statistics.median(early), 1)
    result.update(lag_median=round(statistics.median(lags), 1),
                  lag_p95=round(percentile(lags, 0.95), 1), lag_max=round(max(lags), 1),
                  queued_seconds_max=round(max(c['queued_seconds'] for c in done), 1),
                  rtf_median=round(statistics.median(c['rtf'] for c in done), 3),
                  lag_drift=drift)
    result['ok'] = (result['lag_p95'] <= KEEPUP_P95_SECONDS
                    and result['lag_max'] <= KEEPUP_MAX_SECONDS
                    and (drift is None or drift <= DRIFT_LIMIT_SECONDS))
    return result


def window_stats(windows):
    done = [w for w in windows if w['status'] == 'completed']
    result = {'windows': len(windows), 'completed': len(done), 'failures': len(windows) - len(done)}
    if not done:
        return dict(result, within_limit=False)
    seconds = [w['seconds'] for w in done]
    result.update(seconds_median=round(statistics.median(seconds), 1),
                  seconds_p95=round(percentile(seconds, 0.95), 1),
                  seconds_max=round(max(seconds), 1),
                  wait_seconds_max=round(max(w['wait_seconds'] for w in done), 1),
                  input_tokens_median=round(statistics.median(w['input_tokens'] for w in done)),
                  prompt_tps_median=round(statistics.median(w['prompt_tps'] for w in done), 2),
                  generate_tps_median=round(statistics.median(w['generate_tps'] for w in done), 2),
                  duty_median=round(statistics.median(seconds) / WINDOW_SECONDS, 3))
    result['within_limit'] = result['seconds_max'] <= WINDOW_MAX_SECONDS
    return result


def post_recording_estimate(window_seconds_max):
    """Backlog windows after the recording plus the final integration from the LLM estimate."""
    seconds = round(BACKLOG_WINDOWS * window_seconds_max + FINAL_INTEGRATION_SECONDS, 1)
    return {'seconds': seconds, 'target_seconds': POST_RECORDING_TARGET_SECONDS,
            'backlog_windows': BACKLOG_WINDOWS,
            'final_integration_seconds': FINAL_INTEGRATION_SECONDS,
            'within_target': seconds <= POST_RECORDING_TARGET_SECONDS}


def memory_budget(stt_private_mib, llm_private_mib, gpu_shared_mib, double_counted=False):
    """16 GB estimate: the shared GPU cap and the app budget left after an OS reserve."""
    llm_total = (max(llm_private_mib, gpu_shared_mib) if double_counted
                 else llm_private_mib + gpu_shared_mib)
    total = stt_private_mib + llm_total
    return {'stt_private_mib': round(stt_private_mib, 1),
            'llm_private_mib': round(llm_private_mib, 1),
            'gpu_shared_mib': round(gpu_shared_mib, 1), 'double_counted': double_counted,
            'total_mib': round(total, 1), 'budget_mib': round(APP_BUDGET_MIB, 1),
            'fits_budget': total <= APP_BUDGET_MIB,
            'shared_limit_mib': round(SHARED_GPU_LIMIT_MIB, 1),
            'fits_shared_limit': gpu_shared_mib <= SHARED_GPU_LIMIT_MIB}


def battery_drain(samples, full_mwh):
    """Average drain between the first and last battery capacity samples."""
    if len(samples) < 2 or not full_mwh:
        return None
    first, last = samples[0], samples[-1]
    hours = (last['seconds'] - first['seconds']) / 3600
    used_mwh = first['remaining_mwh'] - last['remaining_mwh']
    if hours <= 0 or used_mwh <= 0:
        return None
    percent_per_hour = used_mwh / full_mwh * 100 / hours
    return {'hours': round(hours, 2), 'used_wh': round(used_mwh / 1000, 2),
            'percent_per_hour': round(percent_per_hour, 1),
            'full_charge_hours': round(100 / percent_per_hour, 1)}


def select_allocation(entries):
    """Fastest drafts among allocations that kept up; near ties go to the lower transcription lag."""
    usable = [e for e in entries if e['keepup']['ok'] and e['windows'].get('seconds_median')]
    if not usable:
        return None
    best = min(e['windows']['seconds_median'] for e in usable)
    close = [e for e in usable
             if e['windows']['seconds_median'] <= best * (1 + ALLOCATION_TIE) + 1e-9]
    chosen = min(close, key=lambda e: e['keepup']['lag_p95'])
    return {'stt_threads': chosen['stt_threads'], 'llm_threads': chosen['llm_threads']}


def suspended(sample_seconds, gap_seconds=SUSPEND_GAP_SECONDS):
    """A gap between counter samples means the machine slept and the run is not judgeable."""
    ordered = sorted(sample_seconds)
    return any(later - earlier > gap_seconds for earlier, later in zip(ordered, ordered[1:]))
```

Run: `python -m unittest discover -s tests -p test_concurrency_metrics.py`
Expected: PASS — `Ran 11 tests`, `OK`

- [ ] **Step 3: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 101 tests`, `OK`

```bash
git add scripts/concurrency_metrics.py tests/test_concurrency_metrics.py
git commit -m "feat: judge transcription lag, draft time and memory budget" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: 동시 실행 측정기

**Files:**
- Create: `scripts/run_concurrency.py`
- Test: `tests/test_run_concurrency.py`
- Modify: `evaluation/README.md` (`## 측정 범위` 바로 앞에 절 추가)

**Interfaces:**
- Consumes: Task 1~5의 모듈, `run_stt_bench.cli_command`·`run_chunk`·`transcript_text`·`load_config`·`BLOCKING_PROCESSES`, `stt_data.FIXTURE_ID`·`chunk_dir`·`verify_chunks`, `prepare_llm.verify`.
- Produces:
  - 상수 `ALLOCATIONS`(STT 8·LLM 4, STT 8·LLM 2, STT 6·LLM 2), `PROBE_CONTEXTS`(4096·8192·16384), `CONTEXT_TOKENS`(8192), `SCREEN_MINUTES`(10), `RUN_MINUTES`(120), `DRAFT_MAX_TOKENS`(300), `BATTERY_FLOOR_PERCENT`(30), `STT_MODEL_ID`(large-v3-turbo), `STT_BUILD`(blas)
  - `draft_payload(transcript, max_tokens=300) -> dict`, `draft_result(response) -> dict`, `schedule(chunks, minutes) -> list[dict]`, `condition_label(power, mode) -> str`, `windows_sample(pid) -> dict`
  - `Session(transcribe, draft, sleep=time.sleep, clock=time.perf_counter, on_update=None)`: `run(items, stop=None) -> (chunks, windows)`. 조각 기록에 `arrival_seconds`·`start_seconds`·`finish_seconds`·`queued_seconds`·`lag_seconds`, 창 기록에 `closed_seconds`·`wait_seconds`·`seconds`·`partial`·`chunks`·`audio_seconds`를 더한다
  - `public_records(records) -> list[dict]`, `render_markdown(summary) -> str`, `verdict(entry, memory, post, suspended_run) -> str`
  - CLI: `--stage {probe,screen,run}`, `--stt-threads`, `--llm-threads`, `--minutes`, `--context-tokens`, `--allow-battery`, `--diagnostic`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_run_concurrency.py`:

```python
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import run_concurrency as R  # noqa: E402

CHUNKS = [{'chunk_id': 'a', 'seconds': 100.0, 'reference': '가'},
          {'chunk_id': 'b', 'seconds': 200.0, 'reference': '나'}]


def transcribed(item, status='completed'):
    return {'chunk_id': item['chunk_id'], 'seconds': item['seconds'], 'status': status,
            'rtf': 0.4, 'transcript': f"{item['chunk_id']} 전사", 'peak_private_mib': 1800.0}


class ScheduleTests(unittest.TestCase):
    def test_arrivals_follow_the_audio_and_stop_at_the_limit(self):
        items = R.schedule(CHUNKS, minutes=5)
        self.assertEqual([(i['chunk_id'], i['arrival_seconds'], i['index']) for i in items],
                         [('a', 100.0, 1), ('b', 300.0, 2)])
        self.assertEqual(len(R.schedule(CHUNKS, minutes=8)), 3)

    def test_condition_label_follows_power_and_mode(self):
        ac = {'ac_power': True, 'battery_percent': 90}
        self.assertEqual(R.condition_label(ac, {'ac_mode': 'best_performance'}),
                         'ac-best_performance')
        self.assertEqual(R.condition_label({'ac_power': False}, {'ac_mode': 'best_performance',
                                                                 'dc_mode': 'best_power_efficiency'}),
                         'battery-best_power_efficiency')


class RequestTests(unittest.TestCase):
    def test_draft_payload_fixes_the_output_size(self):
        payload = R.draft_payload('전사문')
        self.assertEqual((payload['max_tokens'], payload['ignore_eos'], payload['temperature']),
                         (300, True, 0))
        self.assertFalse(payload['cache_prompt'])
        self.assertEqual([m['role'] for m in payload['messages']], ['system', 'user'])
        self.assertEqual(payload['messages'][1]['content'], '전사문')

    def test_draft_result_reads_server_timings(self):
        response = {'choices': [{'message': {'content': '초안 본문'}}],
                    'timings': {'prompt_n': 1900, 'prompt_ms': 3000.0, 'prompt_per_second': 633.3,
                                'predicted_n': 300, 'predicted_ms': 22000.0,
                                'predicted_per_second': 13.6}}
        result = R.draft_result(response)
        self.assertEqual((result['input_tokens'], result['generated_tokens']), (1900, 300))
        self.assertEqual((result['prompt_seconds'], result['generate_seconds']), (3.0, 22.0))
        self.assertEqual((result['prompt_tps'], result['generate_tps']), (633.3, 13.6))
        self.assertEqual(result['output_chars'], 5)


class SampleTests(unittest.TestCase):
    def test_windows_sample_parses_the_powershell_json(self):
        payload = json.dumps({'gpu_shared_bytes': 6 * 1024 * 1024 * 1024, 'discharge_mw': 41198,
                              'remaining_mwh': 45147, 'full_mwh': 52496, 'power_online': 0})
        done = subprocess.CompletedProcess(['powershell'], 0, stdout=payload, stderr='')
        with patch.object(R.subprocess, 'run', return_value=done):
            sample = R.windows_sample(1234)
        self.assertEqual((sample['gpu_shared_mib'], sample['discharge_w']), (6144.0, 41.2))
        self.assertEqual((sample['remaining_mwh'], sample['power_online']), (45147, 0))

    def test_windows_sample_survives_a_failed_call(self):
        with patch.object(R.subprocess, 'run', side_effect=OSError('no powershell')):
            self.assertEqual(R.windows_sample(1234), {})


class SessionTests(unittest.TestCase):
    def session(self, statuses):
        calls = iter(statuses)

        def transcribe(item):
            return transcribed(item, next(calls))

        def draft(job):
            return {'status': 'completed', 'input_tokens': 100, 'prompt_tps': 1.0,
                    'generate_tps': 2.0}

        return R.Session(transcribe, draft, sleep=lambda delay: None)

    def items(self, count, seconds=120.0):
        return [{'chunk_id': f'c{index}', 'seconds': seconds, 'index': index,
                 'arrival_seconds': index * seconds} for index in range(1, count + 1)]

    def test_full_window_and_a_partial_window_at_the_end(self):
        session = self.session(['completed'] * 4)
        chunks, windows = session.run(self.items(4))
        self.assertEqual(len(chunks), 4)
        self.assertEqual([(w['partial'], w['chunks']) for w in windows],
                         [(False, ['c1', 'c2', 'c3']), (True, ['c4'])])
        self.assertEqual(windows[0]['audio_seconds'], 360.0)
        self.assertTrue(all('lag_seconds' in c and 'queued_seconds' in c for c in chunks))

    def test_failed_chunks_stay_out_of_the_draft(self):
        session = self.session(['completed', 'timeout', 'completed', 'completed'])
        chunks, windows = session.run(self.items(4))
        self.assertEqual(sum(1 for c in chunks if c['status'] == 'completed'), 3)
        self.assertEqual([w['chunks'] for w in windows], [['c1', 'c3', 'c4']])

    def test_stop_ends_the_recording_early(self):
        session = self.session(['completed'] * 4)
        chunks, _ = session.run(self.items(4), stop=lambda: True)
        self.assertEqual(chunks, [])


class ReportTests(unittest.TestCase):
    def summary(self):
        return {'runs': [{'stage': 'run', 'condition': 'ac-best_performance', 'stt_threads': 8,
                          'llm_threads': 2,
                          'keepup': {'chunks': 10, 'completed': 10, 'lag_median': 12.0,
                                     'lag_p95': 18.0, 'lag_max': 20.0, 'lag_drift': None,
                                     'ok': True},
                          'windows': {'windows': 2, 'completed': 2, 'seconds_median': 30.0,
                                      'seconds_max': 33.0, 'duty_median': 0.1,
                                      'within_limit': True}}],
                'selected_allocation': {'stt_threads': 8, 'llm_threads': 2},
                'post_recording': {'seconds': 106.3, 'within_target': True},
                'memory': {'total_mib': 9000.0, 'budget_mib': 11980.8, 'gpu_shared_mib': 6000.0,
                           'shared_limit_mib': 8038.4},
                'battery': None, 'verdict': 'feasible_estimate'}

    def test_public_records_drop_transcripts(self):
        records = R.public_records([{'chunk_id': 'a', 'transcript': '내용', 'rtf': 0.4}])
        self.assertEqual(records, [{'chunk_id': 'a', 'rtf': 0.4}])

    def test_render_markdown_lists_runs_and_verdict(self):
        table = R.render_markdown(self.summary())
        self.assertIn('| run | ac-best_performance | 8 | 2 | 10/10 | 12.0 | 18.0 | 20.0 | - | 2/2 '
                      '| 30.0 | 33.0 | 0.100 | 예 | 예 |', table)
        self.assertIn('판정(추정): feasible_estimate', table)

    def test_verdict_requires_every_check(self):
        entry = self.summary()['runs'][0]
        memory = {'fits_budget': True, 'fits_shared_limit': True}
        post = {'within_target': True}
        self.assertEqual(R.verdict(entry, memory, post, False), 'feasible_estimate')
        self.assertEqual(R.verdict(entry, memory, post, True), 'not_judgeable')
        self.assertEqual(R.verdict(entry, dict(memory, fits_shared_limit=False), post, False),
                         'infeasible_estimate')


if __name__ == '__main__':
    unittest.main()
```

Run: `python -m unittest discover -s tests -p test_run_concurrency.py`
Expected: FAIL — `ModuleNotFoundError: No module named 'run_concurrency'`

- [ ] **Step 2: 실행기 구현**

`scripts/run_concurrency.py`:

```python
"""Run STT and LLM together through a simulated lecture recording. Estimates, not app latency."""
import argparse
from datetime import datetime
import itertools
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time

from bench_env import (MEBIBYTE, competing_processes, hide_paths, keep_awake, power_mode,
                       power_status, unique_path, utc_now)
from concurrency_metrics import (battery_drain, closed_window, keepup, memory_budget,
                                 post_recording_estimate, select_allocation, suspended,
                                 window_stats)
from llm_server import LlamaServer
from prepare_llm import verify
from run_stt_bench import (BLOCKING_PROCESSES, cli_command, load_config, run_chunk,
                           transcript_text)
from stt_data import FIXTURE_ID, chunk_dir, verify_chunks

ROOT = Path(__file__).resolve().parents[1]
ALLOCATIONS = [{'stt_threads': 8, 'llm_threads': 4}, {'stt_threads': 8, 'llm_threads': 2},
               {'stt_threads': 6, 'llm_threads': 2}]
PROBE_CONTEXTS = (4096, 8192, 16384)
CONTEXT_TOKENS = 8192
SCREEN_MINUTES, RUN_MINUTES = 10, 120
DRAFT_MAX_TOKENS = 300
BATTERY_FLOOR_PERCENT = 30
MEMORY_SAMPLE_SECONDS = 60
STT_MODEL_ID, STT_BUILD = 'large-v3-turbo', 'blas'
DRAFT_SYSTEM = ('당신은 한국어 강의 노트를 만드는 도우미다. 주어진 전사문에서 요점, 개념과 용어, '
                '예제와 코드, 시험·과제·공지 후보를 뽑아 간결한 구간 초안을 쓴다. '
                '전사문에 없는 내용을 만들지 않는다.')


def draft_payload(transcript, max_tokens=DRAFT_MAX_TOKENS):
    """Fixed-size drafts keep every window the same work; the content is not evaluated."""
    return {'messages': [{'role': 'system', 'content': DRAFT_SYSTEM},
                         {'role': 'user', 'content': transcript}],
            'max_tokens': max_tokens, 'ignore_eos': True, 'temperature': 0, 'seed': 42,
            'cache_prompt': False, 'stream': False}


def draft_result(response):
    timings = response.get('timings') or {}
    content = response['choices'][0]['message']['content']
    return {'status': 'completed', 'input_tokens': timings.get('prompt_n'),
            'generated_tokens': timings.get('predicted_n'),
            'prompt_seconds': round((timings.get('prompt_ms') or 0.0) / 1000, 2),
            'generate_seconds': round((timings.get('predicted_ms') or 0.0) / 1000, 2),
            'prompt_tps': round(timings.get('prompt_per_second') or 0.0, 2),
            'generate_tps': round(timings.get('predicted_per_second') or 0.0, 2),
            'output_chars': len(content)}


def schedule(chunks, minutes):
    """Arrival times as if the audio were recorded live: a chunk arrives when its audio ends."""
    limit, items, elapsed = minutes * 60, [], 0.0
    for chunk in itertools.cycle(chunks):
        elapsed += chunk['seconds']
        if elapsed > limit:
            break
        items.append(dict(chunk, index=len(items) + 1, arrival_seconds=round(elapsed, 3)))
    return items


def condition_label(power, mode):
    on_ac = bool(power.get('ac_power'))
    name = mode.get('ac_mode' if on_ac else 'dc_mode') or 'unknown'
    return f"{'ac' if on_ac else 'battery'}-{name}"


GPU_AND_BATTERY = (
    "$gpu = ((Get-Counter '\\GPU Process Memory(pid_<pid>*)\\Shared Usage' "
    '-ErrorAction SilentlyContinue).CounterSamples '
    '| Measure-Object -Property CookedValue -Sum).Sum; '
    '$b = Get-CimInstance -Namespace root\\wmi -ClassName BatteryStatus '
    '-ErrorAction SilentlyContinue | Select-Object -First 1; '
    '$f = Get-CimInstance -Namespace root\\wmi -ClassName BatteryFullChargedCapacity '
    '-ErrorAction SilentlyContinue | Select-Object -First 1; '
    '@{gpu_shared_bytes = $gpu; discharge_mw = $b.DischargeRate; '
    'remaining_mwh = $b.RemainingCapacity; full_mwh = $f.FullChargedCapacity; '
    'power_online = [int][bool]$b.PowerOnline} | ConvertTo-Json -Compress')


def windows_sample(pid):
    """Shared GPU memory of one process and the battery state, in one PowerShell call."""
    command = ['powershell.exe', '-NoProfile', '-Command',
               GPU_AND_BATTERY.replace('<pid>', str(pid))]
    try:
        done = subprocess.run(command, capture_output=True, text=True, timeout=120,
                              creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        values = json.loads(done.stdout or '{}')
    except (ValueError, OSError, subprocess.SubprocessError):
        return {}
    shared, discharge = values.get('gpu_shared_bytes'), values.get('discharge_mw')
    return {'gpu_shared_mib': round(shared / MEBIBYTE, 1) if shared else None,
            'discharge_w': round(discharge / 1000, 1) if discharge else None,
            'remaining_mwh': values.get('remaining_mwh'), 'full_mwh': values.get('full_mwh'),
            'power_online': values.get('power_online')}


class Session:
    """One paced recording: a transcription worker and a draft worker running side by side."""

    def __init__(self, transcribe, draft, sleep=time.sleep, clock=time.perf_counter,
                 on_update=None):
        self.transcribe, self.draft = transcribe, draft
        self.sleep, self.clock, self.on_update = sleep, clock, on_update
        self.chunks, self.windows, self.pending = [], [], []
        self.chunk_queue, self.draft_queue = queue.Queue(), queue.Queue()
        self.started = None

    def run(self, items, stop=None):
        self.started = self.clock()
        workers = [threading.Thread(target=self.transcribe_loop, daemon=True),
                   threading.Thread(target=self.draft_loop, daemon=True)]
        for worker in workers:
            worker.start()
        for item in items:
            delay = self.started + item['arrival_seconds'] - self.clock()
            if delay > 0:
                self.sleep(delay)
            if stop and stop():
                break
            self.chunk_queue.put(item)
        self.chunk_queue.put(None)
        workers[0].join()
        self.draft_queue.put(None)
        workers[1].join()
        return self.chunks, self.windows

    def elapsed(self):
        return 0.0 if self.started is None else round(self.clock() - self.started, 2)

    def transcribe_loop(self):
        while True:
            item = self.chunk_queue.get()
            if item is None:
                if self.pending:
                    self.queue_window(self.pending, partial=True)
                    self.pending = []
                return
            started = self.elapsed()
            record = self.transcribe(item)
            finished = self.elapsed()
            record.update(arrival_seconds=item['arrival_seconds'], start_seconds=started,
                          finish_seconds=finished,
                          queued_seconds=round(started - item['arrival_seconds'], 2),
                          lag_seconds=round(finished - item['arrival_seconds'], 2))
            self.chunks.append(record)
            if record['status'] == 'completed':
                self.pending.append(record)
                while (closed := closed_window(self.pending)) is not None:
                    self.queue_window(closed, partial=False)
                    self.pending = self.pending[len(closed):]
            if self.on_update:
                self.on_update()

    def queue_window(self, chunks, partial):
        self.draft_queue.put({'chunks': list(chunks), 'partial': partial,
                              'closed_seconds': self.elapsed()})

    def draft_loop(self):
        while True:
            job = self.draft_queue.get()
            if job is None:
                return
            started = self.elapsed()
            record = self.draft(job)
            finished = self.elapsed()
            record.update(closed_seconds=job['closed_seconds'], start_seconds=started,
                          finish_seconds=finished,
                          wait_seconds=round(started - job['closed_seconds'], 2),
                          seconds=round(finished - started, 2), partial=job['partial'],
                          chunks=[chunk['chunk_id'] for chunk in job['chunks']],
                          audio_seconds=round(sum(c['seconds'] for c in job['chunks']), 1))
            self.windows.append(record)
            if self.on_update:
                self.on_update()


def public_records(records):
    """Committed summaries keep timings and sizes, never transcripts."""
    return [{key: value for key, value in record.items() if key != 'transcript'}
            for record in records]


def _cell(value, digits=1):
    if value is None:
        return '-'
    return f'{value:.{digits}f}' if isinstance(value, float) else str(value)


def render_markdown(summary):
    lines = ['| 단계 | 조건 | STT 스레드 | LLM 스레드 | 조각 완료/전체 | 지연 중앙값 | 지연 95% '
             '| 지연 최대 | 지연 추세 | 초안 완료/전체 | 초안 중앙값 | 초안 최대 | 점유율 | 추종 | 초안 한도 |',
             '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- '
             '| --- | --- |']
    for entry in summary.get('runs', []):
        k, w = entry['keepup'], entry['windows']
        lines.append(
            f"| {entry['stage']} | {entry['condition']} | {entry['stt_threads']} "
            f"| {entry['llm_threads']} | {k.get('completed', 0)}/{k.get('chunks', 0)} "
            f"| {_cell(k.get('lag_median'))} | {_cell(k.get('lag_p95'))} | {_cell(k.get('lag_max'))} "
            f"| {_cell(k.get('lag_drift'))} | {w.get('completed', 0)}/{w.get('windows', 0)} "
            f"| {_cell(w.get('seconds_median'))} | {_cell(w.get('seconds_max'))} "
            f"| {_cell(w.get('duty_median'), 3)} | {'예' if k.get('ok') else '아니오'} "
            f"| {'예' if w.get('within_limit') else '아니오'} |")
    memory = summary.get('memory') or {}
    lines += ['', f"선택 스레드 배분: {summary.get('selected_allocation')}",
              f"녹음 종료 후 5분 추정: {summary.get('post_recording')}",
              f"16GB 메모리 추정: 합계 {_cell(memory.get('total_mib'))}MiB / 예산 "
              f"{_cell(memory.get('budget_mib'))}MiB, GPU 공유 {_cell(memory.get('gpu_shared_mib'))}MiB "
              f"/ 한도 {_cell(memory.get('shared_limit_mib'))}MiB",
              f"배터리: {summary.get('battery')}", f"판정(추정): {summary.get('verdict')}"]
    return '\n'.join(lines)


def verdict(entry, memory, post, suspended_run):
    if suspended_run:
        return 'not_judgeable'
    checks = [entry['keepup']['ok'], entry['windows']['within_limit'], post['within_target'],
              memory['fits_budget'], memory['fits_shared_limit']]
    return 'feasible_estimate' if all(checks) else 'infeasible_estimate'


def stt_paths():
    runtime, models = load_config('stt-runtime.json'), load_config('stt-models.json')
    model = next(m for m in models['models'] if m['id'] == STT_MODEL_ID)
    return (ROOT / 'runtimes' / f"whisper-{runtime['tag']}" / STT_BUILD / 'Release/whisper-cli.exe',
            ROOT / 'models/whisper' / model['filename'], model)


def llm_paths():
    model = load_config('llm-model.json')['artifact']
    runtime = load_config('llama-runtime.json')
    return (ROOT / 'runtimes' / runtime['tag'] / 'vulkan' / 'llama-server.exe',
            ROOT / 'models' / model['filename'], model)


def preflight(fixture, allow_battery):
    stt_exe, stt_model_path, stt_model = stt_paths()
    llm_exe, llm_model_path, llm_model = llm_paths()
    problems = []
    if not stt_exe.exists():
        problems.append(f'missing whisper {STT_BUILD} build')
    if not verify(stt_model_path, stt_model['size_bytes'], stt_model['sha256']):
        problems.append(f'STT model {STT_MODEL_ID} missing or corrupt')
    if not llm_exe.exists():
        problems.append('missing llama-server vulkan build')
    if not verify(llm_model_path, llm_model['size_bytes'], llm_model['sha256']):
        problems.append('LLM model missing or corrupt')
    problems += verify_chunks(fixture, chunk_dir())
    if problems:
        raise SystemExit('Preflight failed: ' + '; '.join(problems[:5]) + ' (run scripts/prepare_stt.py, '
                         'scripts/prepare_llm.py and scripts/stt_data.py)')
    if running := competing_processes(BLOCKING_PROCESSES):
        raise SystemExit(f"Stop other inference processes first: {', '.join(running)}")
    power = power_status()
    if power['ac_power'] is not True and not allow_battery:
        raise SystemExit(f'Connect AC power or pass --allow-battery (power: {power})')
    mode = power_mode()
    if mode['ac_mode'] != 'best_performance':
        print(f"Warning: targets are judged in best_performance mode; current: {mode['ac_mode']}",
              flush=True)
    return (stt_exe, stt_model_path), (llm_exe, llm_model_path), power, mode


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', choices=['probe', 'screen', 'run'], default='run')
    parser.add_argument('--stt-threads', type=int, help='required with --stage run')
    parser.add_argument('--llm-threads', type=int, help='required with --stage run')
    parser.add_argument('--minutes', type=float, help='recording length to simulate')
    parser.add_argument('--context-tokens', type=int, default=CONTEXT_TOKENS)
    parser.add_argument('--allow-battery', action='store_true',
                        help='reference runs on battery; results are not used for the verdict')
    parser.add_argument('--diagnostic', action='store_true',
                        help='keep the result in artifacts only')
    args = parser.parse_args()
    if args.stage == 'run' and (args.stt_threads is None or args.llm_threads is None):
        parser.error('--stage run requires --stt-threads and --llm-threads from a screen summary')
    minutes = args.minutes or (SCREEN_MINUTES if args.stage == 'screen' else RUN_MINUTES)

    fixture = json.loads((ROOT / 'evaluation/fixtures' / f'{FIXTURE_ID}.json').read_text('utf-8'))
    (stt_exe, stt_model_path), (llm_exe, llm_model_path), power, mode = preflight(
        fixture, args.allow_battery)
    condition = condition_label(power, mode)
    out_dir = ROOT / 'artifacts' / f'concurrency-{time.time_ns()}'
    out_dir.mkdir(parents=True)
    counters = out_dir / 'counters.csv'
    summary = {'schema_version': 1, 'kind': 'stt-llm-concurrency',
               'scope': 'FLEURS Korean read speech paced like a recording; estimates only',
               'stage': args.stage, 'condition': condition, 'diagnostic': args.diagnostic,
               'minutes': minutes, 'stt_model': STT_MODEL_ID, 'stt_build': STT_BUILD,
               'llm_context_tokens': args.context_tokens, 'fixture_id': FIXTURE_ID,
               'draft_max_tokens': DRAFT_MAX_TOKENS,
               'criteria': {'lag_p95_max': 30.0, 'lag_max': 60.0, 'window_seconds_max': 150.0,
                            'post_recording_target_seconds': 300.0},
               'started_at': utc_now(), 'power_at_start': power, 'power_mode_at_start': mode,
               'probes': [], 'runs': [], 'selected_allocation': None, 'post_recording': None,
               'memory': None, 'memory_if_separate': None, 'battery': None, 'verdict': None}
    save_lock = threading.Lock()

    def save():
        with save_lock:
            (out_dir / 'summary.json').write_text(
                json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')

    def probe():
        for context_tokens in PROBE_CONTEXTS:
            log_path = out_dir / f'probe-{context_tokens}.log'
            with LlamaServer(llm_exe, llm_model_path, log_path, context_tokens=context_tokens,
                             threads=4) as server:
                server.post('/v1/chat/completions', draft_payload('테스트', max_tokens=8))
                record = {'context_tokens': context_tokens,
                          'startup_seconds': server.startup_seconds,
                          **(server.memory() or {}), **windows_sample(server.process.pid)}
            summary['probes'].append(record)
            print(f"[{utc_now()}] probe c{context_tokens}: private "
                  f"{record.get('private_mib')} MiB, gpu shared {record.get('gpu_shared_mib')} MiB",
                  flush=True)
            save()
        folder = out_dir / 'probe-stt'
        folder.mkdir()
        chunk = fixture['chunks'][0]
        prefix = folder / chunk['chunk_id']
        command = cli_command(stt_exe, stt_model_path, chunk_dir() / f"{chunk['chunk_id']}.wav",
                              8, prefix)
        record = run_chunk(command, prefix, chunk)
        summary['stt_probe'] = {key: record.get(key) for key in
                                ('status', 'rtf', 'load_seconds', 'peak_private_mib',
                                 'peak_working_set_mib')}
        chosen = next((p for p in summary['probes']
                       if p['context_tokens'] == args.context_tokens), None)
        if chosen and summary['stt_probe'].get('peak_private_mib'):
            sizes = (summary['stt_probe']['peak_private_mib'], chosen.get('private_mib') or 0.0,
                     chosen.get('gpu_shared_mib') or 0.0)
            summary['memory'] = memory_budget(*sizes, double_counted=True)
            summary['memory_if_separate'] = memory_budget(*sizes)
        print(f"[{utc_now()}] probe stt: {summary['stt_probe']}", flush=True)
        save()

    def run_allocation(stage, stt_threads, llm_threads, run_minutes):
        folder = out_dir / f'{stage}-stt{stt_threads}-llm{llm_threads}'
        folder.mkdir()
        entry = {'stage': stage, 'condition': condition, 'stt_threads': stt_threads,
                 'llm_threads': llm_threads, 'minutes': run_minutes, 'keepup': {}, 'windows': {},
                 'llm_memory': [], 'records': {'chunks': [], 'windows': []}}
        summary['runs'].append(entry)
        items = schedule(fixture['chunks'], run_minutes)
        stop_flag = threading.Event()
        with LlamaServer(llm_exe, llm_model_path, folder / 'server.log',
                         context_tokens=args.context_tokens, threads=llm_threads) as server:
            entry['llm_startup_seconds'] = server.startup_seconds

            def transcribe(item):
                prefix = folder / f"{item['index']:04d}-{item['chunk_id']}"
                wav = chunk_dir() / f"{item['chunk_id']}.wav"
                record = run_chunk(cli_command(stt_exe, stt_model_path, wav, stt_threads, prefix),
                                   prefix, item)
                if record['status'] == 'completed':
                    result = json.loads(Path(f'{prefix}.json').read_text('utf-8'))
                    record['transcript'] = transcript_text(result)
                return record

            def draft(job):
                transcript = ' '.join(c.get('transcript', '') for c in job['chunks']).strip()
                try:
                    return draft_result(server.post('/v1/chat/completions',
                                                    draft_payload(transcript)))
                except (RuntimeError, OSError) as error:
                    return {'status': 'failed', 'error': hide_paths(str(error))}

            def sample_memory():
                while True:
                    entry['llm_memory'].append({'at': utc_now(), 'seconds': session.elapsed(),
                                                **(server.memory() or {}),
                                                **windows_sample(server.process.pid)})
                    save()
                    if stop_flag.wait(MEMORY_SAMPLE_SECONDS):
                        return

            def update():
                chunks, windows = list(session.chunks), list(session.windows)
                entry['records'] = {'chunks': public_records(chunks), 'windows': windows}
                entry['keepup'] = keepup(chunks, run_minutes)
                entry['windows'] = window_stats(windows)
                save()

            def battery_stop():
                if power['ac_power']:
                    return False
                return (power_status().get('battery_percent') or 100) <= BATTERY_FLOOR_PERCENT

            session = Session(transcribe, draft, on_update=update)
            sampler = threading.Thread(target=sample_memory, daemon=True)
            print(f'[{utc_now()}] {stage} stt{stt_threads}/llm{llm_threads} {condition}: '
                  f'{len(items)} chunks over {run_minutes} min', flush=True)
            sampler.start()
            try:
                session.run(items, stop=battery_stop)
            finally:
                stop_flag.set()
                sampler.join(timeout=180)
            update()
        k, w = entry['keepup'], entry['windows']
        print(f"[{utc_now()}] {stage} stt{stt_threads}/llm{llm_threads}: "
              f"lag p95 {k.get('lag_p95')}s max {k.get('lag_max')}s, "
              f"drafts {w.get('completed')}/{w.get('windows')} median {w.get('seconds_median')}s",
              flush=True)
        return entry

    def finish(entry):
        chunks = [c for c in entry['records']['chunks'] if c.get('peak_private_mib')]
        stt_private = max((c['peak_private_mib'] for c in chunks), default=0.0)
        llm_private = max((m.get('private_mib') or 0.0 for m in entry['llm_memory']), default=0.0)
        gpu_shared = max((m.get('gpu_shared_mib') or 0.0 for m in entry['llm_memory']), default=0.0)
        # Vulkan allocations show up in both numbers, so the shared part is counted once.
        summary['memory'] = memory_budget(stt_private, llm_private, gpu_shared, double_counted=True)
        summary['memory_if_separate'] = memory_budget(stt_private, llm_private, gpu_shared)
        summary['post_recording'] = post_recording_estimate(entry['windows'].get('seconds_max') or 0.0)
        battery_samples = [{'seconds': m['seconds'], 'remaining_mwh': m['remaining_mwh']}
                           for m in entry['llm_memory'] if m.get('remaining_mwh')
                           and not m.get('power_online')]
        full = next((m.get('full_mwh') for m in entry['llm_memory'] if m.get('full_mwh')), None)
        summary['battery'] = battery_drain(battery_samples, full)
        samples = [m['seconds'] for m in entry['llm_memory']]
        summary['suspend_suspect'] = suspended(samples, MEMORY_SAMPLE_SECONDS * 3)
        summary['verdict'] = verdict(entry, summary['memory'], summary['post_recording'],
                                     summary['suspend_suspect'])

    sampler_process = subprocess.Popen(
        ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
         str(ROOT / 'scripts/sample_counters.ps1'), '-Csv', str(counters),
         '-WatchPid', str(os.getpid())],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    status = 'completed'
    try:
        with keep_awake():
            if args.stage == 'probe':
                probe()
            elif args.stage == 'screen':
                for allocation in ALLOCATIONS:
                    run_allocation('screen', allocation['stt_threads'], allocation['llm_threads'],
                                   minutes)
                summary['selected_allocation'] = select_allocation(summary['runs'])
                if summary['selected_allocation'] is None:
                    status = 'failed'
                    summary['error'] = 'No thread allocation kept transcription up to date'
            else:
                finish(run_allocation('run', args.stt_threads, args.llm_threads, minutes))
                summary['selected_allocation'] = {'stt_threads': args.stt_threads,
                                                  'llm_threads': args.llm_threads}
    finally:
        summary.update(status=status, finished_at=utc_now(), power_at_end=power_status(),
                       power_mode_at_end=power_mode())
        save()
        sampler_process.terminate()
    (out_dir / 'report.md').write_text(render_markdown(summary) + '\n', 'utf-8')
    print(render_markdown(summary), flush=True)
    if args.diagnostic:
        print('Diagnostic run: results kept in artifacts only', flush=True)
    else:
        stamp = datetime.now().strftime('%Y-%m-%d')
        name = f'{stamp}-concurrency-{args.stage}-{condition}.json'
        result_path = unique_path(ROOT / 'evaluation/results' / name)
        result_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')
        print(f'Summary: {result_path}', flush=True)
    print(f'Logs: {out_dir}', flush=True)
    if status != 'completed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
```

Run: `python -m unittest discover -s tests -p test_run_concurrency.py`
Expected: PASS — `Ran 12 tests`, `OK`

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 113 tests`, `OK`

Run: `python scripts/run_concurrency.py --stage run`
Expected: 종료 코드 2, `error: --stage run requires --stt-threads and --llm-threads from a screen summary`

- [ ] **Step 3: 5분 진단 실행으로 전체 흐름 확인**

노트북을 쓰는 중에도 할 수 있는 짧은 확인이다. 배터리 사용 중이면 `--allow-battery`를 붙인다.

Run: `python scripts/run_concurrency.py --stage run --stt-threads 8 --llm-threads 2 --minutes 5 --diagnostic`

Expected: 다음 형식의 줄이 나오고 종료 코드 0이다.

```
[...] run stt8/llm2 ac-best_performance: 13 chunks over 5 min
[...] run stt8/llm2: lag p95 ...s max ...s, drafts 2/2 median ...s
| 단계 | 조건 | STT 스레드 | ... |
...
판정(추정): ...
Diagnostic run: results kept in artifacts only
Logs: ...\artifacts\concurrency-...
```

`artifacts/concurrency-*/summary.json`에서 다음을 확인한다.
- `runs[0].records.chunks`에 `lag_seconds`·`queued_seconds`·`peak_private_mib`가 있고 전사문(`transcript`)은 없다.
- `runs[0].records.windows`의 창에 `input_tokens`와 `generated_tokens`(300)가 있다. 5분 실행은 음성이 300초에 못 미쳐 창 1개가 `partial: true`로 끝난다.
- `runs[0].llm_memory`에 `private_mib`와 `gpu_shared_mib`가 있다.
- `counters.csv`에 5초 간격 기록이 쌓였다.

- [ ] **Step 4: 사용법 문서화**

`evaluation/README.md`의 `## 측정 범위` 바로 앞에 다음 절을 추가한다.

````markdown
## 녹음 중 STT·LLM 동시 실행 측정

```powershell
python scripts/run_concurrency.py --stage probe
python scripts/run_concurrency.py --stage screen
python scripts/run_concurrency.py --stage run --stt-threads 8 --llm-threads 2 --minutes 120
```

- `run_concurrency.py`는 FLEURS 조각을 녹음 속도로 도착시키고, 전사와 5분 창 구간 초안을 동시에 실행한다. 전사는 whisper.cpp large-v3-turbo f16(OpenBLAS), 초안은 llama-server(Vulkan)로 만든다.
- `probe`는 컨텍스트 4,096·8,192·16,384의 서버 메모리와 STT 조각 1개의 최대 메모리를 잰다. `screen`은 스레드 배분 3가지를 10분씩 비교한다. `run`은 고른 배분으로 길게 측정한다.
- 판정 기준은 전사 지연 95번째 백분위 30초·최대 60초, 구간 초안 최대 150초, 녹음 종료 후 5분 재추정 300초, 16GB 메모리 예산이다. 판정은 AC 전원·"최고 성능"에서만 한다.
- 참고 측정은 `--allow-battery`로 배터리 조건에서 실행한다. 배터리 잔량이 30% 이하가 되면 새 조각 도착을 멈추고 정상 종료한다.
- 결과는 `evaluation/results/<날짜>-concurrency-<단계>-<조건>.json`에 저장하고, 원출력·서버 로그·카운터 CSV는 `artifacts/`에만 둔다. `--diagnostic`을 쓴 실행은 저장하지 않는다.
- 이 측정은 낭독 음성을 녹음처럼 흘려보낸 추정이며, 실제 강의 녹음·요약 품질·앱 지연시간을 검증하지 않는다.

````

- [ ] **Step 5: 커밋**

```bash
git add scripts/run_concurrency.py tests/test_run_concurrency.py evaluation/README.md
git commit -m "feat: run STT and LLM together through a paced recording" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: 실측과 판정 기록

**Files:**
- Create: `evaluation/results/<날짜>-concurrency-probe-<조건>.json`, `...-screen-...json`, `...-run-...json` (실행기가 생성)
- Create: `docs/validation/<날짜>-concurrency.md`, `docs/decisions/0009-concurrent-processing.md`
- Modify: `docs/ROADMAP.md`, `docs/PRD.md`, `docs/decisions/0006-lecture-first-product-scope.md`, `README.md`

**Interfaces:**
- Consumes: Task 1~6의 도구.
- Produces: 동시 실행 가능 여부(추정), 스레드 배분, 16GB 메모리 판단, 발열·배터리 기록, 순차 모드 전환 조건.

- [ ] **Step 1: 사용자 확인과 사전 점검**

판정 측정은 약 2.5시간(배분 비교 30분 + 본 측정 2시간), 참고 측정은 약 1.5시간이다. 시작 전에 사용자에게 확인받는다: AC 전원 연결, 전원 모드 "최고 성능", 절전 전환 "안 함", 무거운 앱 종료, OneDrive 종료 허락.

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 113 tests`, `OK`

Run: `python -c "import sys; sys.path.insert(0,'scripts'); import bench_env as E; print(E.power_status(), E.power_mode()['ac_mode'], E.competing_processes({'llama-server.exe','llama-bench.exe','llama-cli.exe','whisper-cli.exe','whisper-server.exe'}))"`
Expected: `{'ac_power': True, ...} best_performance []`

Run: `powershell -NoProfile -Command "powercfg /q SCHEME_CURRENT SUB_SLEEP STANDBYIDLE | Select-Object -Last 2"`
Expected: AC 전원 설정 색인이 `0x00000000`(절전 전환 없음)

OneDrive 종료와 시작 직전 부하 기록은 STT 측정과 같은 방법을 쓴다([STT 실행 계획](2026-09-17-stt-benchmark.md) Task 6 Step 1).

- [ ] **Step 2: 메모리 확인과 스레드 배분 비교**

Run: `python scripts/run_concurrency.py --stage probe`
Expected: 컨텍스트별 `private ... MiB, gpu shared ... MiB` 3줄과 `probe stt: {...}` 1줄, 마지막에 `Summary: ...json`. 약 5분.

Run: `python scripts/run_concurrency.py --stage screen`
Expected: 배분 3가지의 시작·완료 줄과 표, `선택 스레드 배분: {'stt_threads': ..., 'llm_threads': ...}`. 약 35분(서버 재시작 포함).

배분이 선택되지 않으면(`No thread allocation kept transcription up to date`) 종료 코드 1이다. 이때는 2시간 측정을 시작하지 말고 사용자에게 결과를 보고한 뒤 결정을 받는다.

- [ ] **Step 3: 판정 측정(AC·최고 성능, 2시간)**

Run:

```bash
python scripts/run_concurrency.py --stage run --stt-threads <선택> --llm-threads <선택> --minutes 120 > artifacts/concurrency-console.log 2>&1; echo "exit $?" >> artifacts/concurrency-console.log
```

측정 중에는 노트북을 쓰지 않는다. 판단 규칙은 다음과 같다.
- `exit 0`이면 Step 4로 간다.
- 사전 점검 실패는 측정 전 중단이다. 원인을 해결하고 다시 실행한다.
- 서버가 죽어 실패하면 요약의 `error`와 `artifacts/concurrency-*/run-*/server.log`를 확인하고, 원인이 일시적이면 한 번만 다시 실행한다.
- `verdict`가 `not_judgeable`이면 대기 모드로 중단된 것이다. 절전 설정을 확인하고 다시 측정한다.

실행 중 발견(2026-09-23): 2시간 판정 측정에서 llama-server의 전용 메모리가 초안 1건마다 약 180MiB씩 늘어 7.5GiB에서 11.0GiB가 됐고, 16GB 예산 판정이 실패로 나왔다. 원인은 `llama-server`가 끝난 요청의 KV 상태를 프롬프트 캐시(`--cache-ram`, 기본 8,192MiB)에 저장하는 동작이다. 같은 전사문을 반복하면 늘지 않고 새 전사문마다 늘었으며, 캐시를 끄면(`--cache-ram 0`) 요청당 증가가 0.4MiB로 사라지고 초안 시간은 25~27초로 같았다. `llm_server.server_command`에 `cache_ram` 인자(기본 0)를 더하고 실행기에 `--cache-ram` 옵션과 요약 항목 `llm_cache_ram_mib`를 추가했다(테스트 115개).

- [ ] **Step 4: 참고 측정**

전원 모드를 "최고의 전원 효율성"으로 바꿔 달라고 요청한 뒤 실행한다.

Run: `python scripts/run_concurrency.py --stage run --stt-threads <선택> --llm-threads <선택> --minutes 30`

그 다음 AC를 뽑고 전원 모드를 "최고 성능"으로 되돌려 달라고 요청한 뒤 실행한다.

Run: `python scripts/run_concurrency.py --stage run --stt-threads <선택> --llm-threads <선택> --minutes 60 --allow-battery`

Expected: 조건 이름이 각각 `ac-best_power_efficiency`, `battery-best_performance`로 붙은 요약 파일 2개. 배터리 실행은 잔량 30%에서 일찍 끝날 수 있다.

측정이 끝나면 OneDrive를 다시 실행하고 사용자에게 전원 설정 복구를 알린다.

- [ ] **Step 5: 결과 확인**

Run:

```bash
python - <<'EOF'
import json, pathlib
for path in sorted(pathlib.Path('evaluation/results').glob('*concurrency*.json')):
    s = json.loads(path.read_text('utf-8'))
    print(path.name, s['stage'], s['condition'], s.get('verdict'))
    for entry in s['runs']:
        k, w = entry['keepup'], entry['windows']
        print('  ', entry['stt_threads'], entry['llm_threads'],
              {x: k.get(x) for x in ('completed', 'chunks', 'lag_p95', 'lag_max', 'lag_drift', 'ok')},
              {x: w.get(x) for x in ('completed', 'windows', 'seconds_median', 'seconds_max', 'duty_median')})
    print('  memory', s.get('memory'), 'post', s.get('post_recording'), 'battery', s.get('battery'))
EOF
```

Expected: 단계·조건별 한 줄과 판정. 표는 `artifacts/concurrency-*/report.md`에 있다. 온도·전력은 `artifacts/concurrency-*/counters.csv`에서 요약한다(STT 측정과 같은 방식).

- [ ] **Step 6: 보고서 작성**

`docs/validation/<날짜>-concurrency.md`를 다음 구조로 쓴다. 수치는 요약 JSON과 `report.md`에서 옮기고 추정값에는 "추정"을 붙인다.

````markdown
# 녹음 중 STT·LLM 동시 실행 측정

- 날짜, 범위(낭독 음성 재현, 실제 강의 아님), 판정 조건, 결과 파일 링크
## 1. 요약
동시 처리 가능 여부(추정), 선택한 스레드 배분, 16GB 판단, 발열·배터리 요점을 3~5문장으로.
## 2. 측정 조건
기기, 전원 상태·모드, 절전 설정, 배경 부하, 실행 시간, 온도·전력, 모델·빌드·컨텍스트, 조각 구성.
## 3. 메모리 확인
컨텍스트별 서버 메모리와 GPU 공유 메모리, STT 조각 메모리, 16GB 예산 비교. 전용 메모리와 GPU 공유가 겹쳐 잡히는 사실을 적는다.
## 4. 스레드 배분 비교
배분 3가지의 전사 지연과 초안 시간 표, 선택 근거.
## 5. 판정 측정(2시간)
전사 지연 분포와 추세, 초안 시간·점유율, 종료 후 5분 재추정, 실패·시간 초과, 온도·전력.
## 6. 참고 조건
효율 모드와 배터리 조건의 같은 지표, 배터리 소모율과 예상 지속 시간.
## 7. 판정과 다음 결정
동시 실행 판정(추정), 순차 모드 전환 조건 제안, 16GB 실기기 확인 필요성.
## 8. 한계
낭독 음성, 합성 조각, 고정 출력량, 기기 1대·1회 측정, 통제하지 않은 배경 부하, 16GB 미검증, 오디오 캡처 부하 제외.
````

- [ ] **Step 7: 결정 기록과 문서 갱신**

`docs/decisions/0009-concurrent-processing.md`를 만든다. 상태는 "초안: 공개 낭독 음성 재현 기준, 실제 강의·16GB 기기 검증 전"으로 둔다. 내용은 동시 실행 방식(전사 우선, 창 초안), 스레드 배분과 컨텍스트, 16GB 판단, 순차 모드·배터리 전환 조건, 재검토 조건이다.

`docs/ROADMAP.md`:
- M0 남은 작업 3 끝에 `결과(추정): <판정>, [보고서](validation/<날짜>-concurrency.md), [결정 초안](decisions/0009-concurrent-processing.md).`를 붙인다. 실제 강의·16GB 기기 확인이 남았으면 체크박스는 `[ ]`로 둔다.
- `## 7. 바로 다음 작업`을 판정에 따라 교체한다. 동시 실행이 가능하면 남은 작업 4(수직 흐름 검증)와 7(앱 기술 검증)을, 불가능하면 순차 모드 설계 결정을 다음으로 둔다.

`docs/PRD.md`의 `녹음 중 처리` 수용 기준 줄에 측정 결과 링크를 붙이고, `docs/decisions/0006-lecture-first-product-scope.md` 5절의 동시 실행 위험 항목에 결과를 적는다. `README.md` 문서 목록에 `- [녹음 중 STT·LLM 동시 실행 측정](docs/validation/<날짜>-concurrency.md)`을 추가한다.

- [ ] **Step 8: 검증 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 113 tests`, `OK`

Run: `python artifacts/check_docs.py`(STT 측정에서 쓴 검사 스크립트. 없으면 [STT 실행 계획](2026-09-17-stt-benchmark.md) Task 6 Step 6의 내용으로 다시 만든다)
Expected: `local paths: none`, `broken links: none`

Run: `git status --short`
Expected: 이번 Task의 결과·문서 파일만 변경되어 있고 `artifacts/`·`models/`·`runtimes/`는 표시되지 않는다.

```bash
git add evaluation/results/<날짜>-concurrency-*.json docs/validation/<날짜>-concurrency.md docs/decisions/0009-concurrent-processing.md docs/ROADMAP.md docs/PRD.md docs/decisions/0006-lecture-first-product-scope.md README.md
git commit -m "test: measure STT and LLM running together during recording" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 9: 사용자 보고**

작업·결과·다음 형식으로 보고한다. 결과에는 판정(추정), 전사 지연과 초안 시간, 16GB 메모리 판단, 배터리 소모율, 순차 모드 전환 조건 제안을 포함한다. 판정이 부정적이면 선택지를 제시하고 사용자 결정을 기다린다.
