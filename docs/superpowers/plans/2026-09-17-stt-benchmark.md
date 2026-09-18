# 로컬 STT(whisper.cpp CPU) 측정 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 고정된 whisper.cpp `b5130` CPU·OpenBLAS 빌드로 Whisper 모델 6종을 FLEURS 한국어 30초 조각에서 측정해, 녹음 중 CPU 실시간 전사 가능 여부와 A 단계 1차 STT 후보를 추정한다.

**Architecture:** 공통 환경 점검(`bench_env.py`)을 llama 측정기에서 분리해 두 측정기가 함께 쓴다. 다운로드 준비(`prepare_stt.py`), 조각 생성(`stt_data.py`), 오류율·시간 해석(`stt_metrics.py`), 측정 실행기(`run_stt_bench.py`)를 표준 라이브러리로 구현하고, 조각마다 `whisper-cli`를 네트워크 없이 실행한다.

**Tech Stack:** Python 3.11 표준 라이브러리(`unittest`, `subprocess`, `wave`, `tarfile`, `array`, `winreg`, `ctypes`), whisper.cpp `b5130`(`whisper-cli.exe`), ggml Whisper 모델, Google FLEURS 한국어 test.

## Global Constraints

- 설계 문서: `docs/superpowers/specs/2026-09-17-stt-benchmark-design.md`. 판정은 추정이며 앱 지연시간·출시 품질 판정이 아니다.
- Python 3.11 이상 표준 라이브러리만 사용한다. 새 의존성을 추가하지 않는다. 전체 테스트 명령은 `python -m unittest discover -s tests -v`다.
- 시간 목표 판정 조건은 AC 전원·Windows 전원 모드 "최고 성능"이다. 전원 설정·드라이버·레지스트리는 읽기만 하고 바꾸지 않는다. 절전 방지는 측정 프로세스 실행 중에만 요청한다.
- 속도 기준: 조각 RTF 중앙값 ≤ 0.5, 95번째 백분위(최근접 순위) ≤ 0.8. 실패 조각이 10%를 넘으면 판정 불가. 1차 후보는 속도 기준 통과 모델 중 CER 최저, CER 차이 0.005 이내면 RTF 중앙값이 낮은 모델.
- 비교 실행은 `-nt`, CER 상위 2개 모델만 타임스탬프 방식으로 재측정한다. 조각별 시간 제한 300초.
- 모델·빌드·FLEURS 원본·조각 WAV·원출력은 `models/`, `runtimes/`, `downloads/`, `artifacts/`에만 두고 커밋하지 않는다. 커밋하는 요약·조각 목록에는 로컬 경로를 남기지 않는다.
- 공개 데이터(FLEURS, CC BY 4.0)만 사용한다. 강의 녹음은 이 계획에서 다루지 않는다.
- 문서는 한국어, 코드·주석은 영어로 작성한다. 각 Task는 검증 후 해당 경로만 stage하여 main에 커밋하고, 메시지 끝에 `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`를 붙인다. push하지 않는다.

---

## 사전 확인 사실 (2026-09-17, 계획 작성 중 직접 확인)

- whisper.cpp 최신 정식 릴리스 `v1.9.4`(2026-09-11)에는 실행 파일이 없고, 같은 커밋(`927cfce3`)의 사전 릴리스 빌드 태그 `b5130`에 Windows x64 CPU(`whisper-bin-x64.zip`, 8,573,270 bytes)와 OpenBLAS(`whisper-blas-bin-x64.zip`, 21,360,234 bytes) 빌드가 있다. Vulkan 빌드는 없고, 그 밖의 x64 가속 빌드는 NVIDIA CUDA용이다(기준 기기 GPU는 Intel Arc 140V). 저장소 라이선스는 MIT다.
- 두 zip은 `Release/` 폴더 안에 `whisper-cli.exe`와 DLL을 담고 있다. OpenBLAS 빌드는 `ggml-blas.dll`, `libopenblas.dll`을 추가로 담는다.
- `whisper-cli` 기본값: 4스레드, 빔 5·후보 5, 언어 `en`, 온도 대체 사용. `-oj`·`-of`·`-nt`·`-l`·`-t` 옵션을 확인했다. 표준 오류에 `whisper_print_timings:     load time =   269.30 ms`, `total time =  7669.65 ms`와 `4 threads, 1 processors, 5 beams + best of 5, lang = ko` 형식의 줄이 나온다. JSON 출력은 `transcription[].text`를 담는다.
- FLEURS `ko_kr` test는 382줄, 7열(문장 번호, 파일명, 원문, 정규화 전사, 글자 분해, 샘플 수, 성별), 고유 문장 270개, 1.33시간이다. 음성 382개는 모두 모노·16kHz·32비트 부동소수점 WAV(`fact` 청크 포함)이며 tar 안에 `test/<파일명>`으로 들어 있다.
- 조각 규칙으로 만든 결과: 조각 60개, 문장 106개, 총 1,406초, 조각당 13.2~29.9초.
- small q5_1 실행 확인(노트북 사용 중, 4스레드): 조각 002~006 합계 CER은 타임스탬프 방식 0.419, `-nt` 0.227이었다. `-nt`에서도 조각 002(0.635)·006(0.319)은 뒷부분이 빠졌다.
- 이 계획의 코드는 저장소 사본에서 먼저 구현했고, 기존 테스트를 포함한 80개가 통과했다. 실제 실행 파일·모델·조각으로 `run_chunk` 경로를 확인했다.
- 계획 작성 중 동의 범위 안에서 빌드 zip 2개, FLEURS 파일 2개, small q5_1 모델을 받아 두었다. 모두 크기·SHA-256이 설정값과 일치하며, `prepare_stt.py`는 기존 파일을 검증만 하고 넘어간다.

## File Structure

| 경로 | 구분 | 책임 |
| --- | --- | --- |
| `scripts/bench_env.py` | 생성 | 전원 상태·전원 모드 읽기, 경쟁 프로세스 확인, 절전 방지, 경로 가림, 덮어쓰지 않는 경로, UTC 시각 |
| `scripts/run_llama_bench.py` | 수정 | `bench_env` 사용, 요약에 전원 모드 기록, whisper 프로세스도 경쟁 프로세스로 취급 |
| `scripts/prepare_llm.py` | 수정 | `safe_extract_zip` 분리 |
| `scripts/prepare_stt.py` | 생성 | STT 빌드·모델·FLEURS 다운로드와 검증 |
| `scripts/stt_metrics.py` | 생성 | 정규화, 편집 거리, CER, 이상 의심, whisper 시간 기록, 백분위 |
| `scripts/stt_data.py` | 생성 | FLEURS 조각 계획·WAV 변환·조각 목록·검증 |
| `scripts/run_stt_bench.py` | 생성 | 실행 설정 선정·모델 비교·타임스탬프 비교, 요약·표 작성 |
| `config/stt-runtime.json`, `config/stt-models.json`, `config/stt-eval-data.json` | 생성 | 고정 파일의 크기·해시·라이선스 |
| `tests/test_bench_env.py`, `tests/test_prepare_stt.py`, `tests/test_stt_metrics.py`, `tests/test_stt_data.py`, `tests/test_run_stt_bench.py` | 생성 | 새 모듈 테스트 |
| `tests/test_run_llama_bench.py`, `tests/test_prepare_llm.py` | 수정 | 이동한 테스트 정리, zip 해제 테스트 추가 |
| `evaluation/fixtures/stt-fleurs-ko-v1.json` | 생성 | 조각 목록·정답·출처 |
| `evaluation/README.md` | 수정 | STT 측정 사용법 |
| `evaluation/results/<날짜>-stt-bench-all.json` | 측정 후 생성 | 요약 |
| `docs/validation/<날짜>-stt-bench.md`, `docs/decisions/0008-stt-candidate.md` | 측정 후 생성 | 보고서, STT 1차 후보 결정 초안 |

`<날짜>`는 측정을 실행한 날짜(`YYYY-MM-DD`)이며 실행기가 출력하는 `Summary:` 경로의 날짜와 같다.

---

### Task 1: 공통 환경 점검 모듈 분리

**Files:**
- Create: `scripts/bench_env.py`
- Test: `tests/test_bench_env.py`
- Modify: `scripts/run_llama_bench.py` (전체 교체)
- Modify: `tests/test_run_llama_bench.py` (전체 교체)

**Interfaces:**
- Consumes: 없음.
- Produces (`bench_env`):
  - `ROOT: Path`, `POWER_MODES: dict[str, str]`
  - `utc_now() -> str`
  - `running_processes(tasklist_csv: str, blocked: set[str]) -> list[str]`, `competing_processes(blocked: set[str]) -> list[str]`
  - `describe_power(ac_line, battery_percent, status_flag) -> dict`, `power_status() -> dict`: 키 `ac_power`, `battery_percent`, `battery_saver`
  - `describe_power_mode(ac_overlay, dc_overlay) -> dict`, `power_mode() -> dict`: 키 `ac_overlay`, `ac_mode`, `dc_overlay`, `dc_mode`(`best_performance`·`best_power_efficiency`·`unknown`·None)
  - `keep_awake()` 컨텍스트 관리자, `hide_paths(text: str) -> str`(`<repo>`, `<home>`), `unique_path(path: Path) -> Path`
- `run_llama_bench.BLOCKING_PROCESSES`에 `whisper-cli.exe`, `whisper-server.exe`를 추가하고, 요약에 `power_mode_at_start`·`power_mode_at_end`를 기록한다.

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_bench_env.py`:

```python
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

    def test_keep_awake_restores_normal_sleep(self):
        with patch.object(E.ctypes.windll.kernel32, 'SetThreadExecutionState') as api:
            with E.keep_awake():
                api.assert_called_once_with(0x80000001)
            api.assert_called_with(0x80000000)

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
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_bench_env.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'bench_env'`

- [ ] **Step 3: 모듈 구현**

`scripts/bench_env.py`:

```python
"""Shared benchmark environment checks. Reads power state; never changes system settings."""
from contextlib import contextmanager
import csv
import ctypes
from datetime import datetime, timezone
import io
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ES_CONTINUOUS, ES_SYSTEM_REQUIRED = 0x80000000, 0x00000001
OVERLAY_KEY = r'SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes'
POWER_MODES = {'961cc777-2547-4f9d-8174-7d86181b8a7a': 'best_power_efficiency',
               'ded574b5-45a0-4f42-8737-46345c09c238': 'best_performance'}


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds')


def running_processes(tasklist_csv, blocked):
    found = set()
    for row in csv.reader(io.StringIO(tasklist_csv)):
        if row and row[0].lower() in blocked:
            found.add(row[0].lower())
    return sorted(found)


def competing_processes(blocked):
    output = subprocess.run(['tasklist', '/FO', 'CSV', '/NH'], capture_output=True, text=True,
                            encoding='oem', errors='replace', check=True).stdout
    return running_processes(output, blocked)


def describe_power(ac_line, battery_percent, status_flag):
    return {'ac_power': {0: False, 1: True}.get(ac_line),
            'battery_percent': None if battery_percent == 255 else battery_percent,
            'battery_saver': bool(status_flag & 1)}


class _PowerStatus(ctypes.Structure):
    _fields_ = [('ACLineStatus', ctypes.c_ubyte), ('BatteryFlag', ctypes.c_ubyte),
                ('BatteryLifePercent', ctypes.c_ubyte), ('SystemStatusFlag', ctypes.c_ubyte),
                ('BatteryLifeTime', ctypes.c_ulong), ('BatteryFullLifeTime', ctypes.c_ulong)]


def power_status():
    status = _PowerStatus()
    if not ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(status)):
        return {'ac_power': None, 'battery_percent': None, 'battery_saver': None}
    return describe_power(status.ACLineStatus, status.BatteryLifePercent,
                          status.SystemStatusFlag)


def describe_power_mode(ac_overlay, dc_overlay):
    def name(guid):
        return None if guid is None else POWER_MODES.get(guid.lower(), 'unknown')
    return {'ac_overlay': ac_overlay, 'ac_mode': name(ac_overlay),
            'dc_overlay': dc_overlay, 'dc_mode': name(dc_overlay)}


def power_mode():
    """Read the Windows power-mode overlay GUIDs from the registry (read-only)."""
    import winreg
    values = {}
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, OVERLAY_KEY) as key:
            for value in ('ActiveOverlayAcPowerScheme', 'ActiveOverlayDcPowerScheme'):
                try:
                    values[value] = winreg.QueryValueEx(key, value)[0]
                except FileNotFoundError:
                    values[value] = None
    except OSError:
        return describe_power_mode(None, None)
    return describe_power_mode(values['ActiveOverlayAcPowerScheme'],
                               values['ActiveOverlayDcPowerScheme'])


@contextmanager
def keep_awake():
    """Block idle sleep while this process runs; system power settings stay unchanged."""
    kernel32 = ctypes.windll.kernel32
    kernel32.SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)
    try:
        yield
    finally:
        kernel32.SetThreadExecutionState(ES_CONTINUOUS)


def hide_paths(text):
    """Mask the checkout and home directories so logs are safe to commit."""
    replacements = [(str(ROOT), '<repo>'), (ROOT.as_posix(), '<repo>'),
                    (str(Path.home()), '<home>'), (Path.home().as_posix(), '<home>')]
    for local, mask in sorted(replacements, key=lambda item: -len(item[0])):
        text = text.replace(local, mask)
    return text


def unique_path(path):
    candidate, index = path, 2
    while candidate.exists():
        candidate = path.with_name(f'{path.stem}-{index}{path.suffix}')
        index += 1
    return candidate
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_bench_env.py -v`
Expected: PASS — `Ran 6 tests`, `OK`

- [ ] **Step 5: llama 측정기가 공통 모듈을 쓰도록 교체**

`scripts/run_llama_bench.py` 전체를 다음으로 바꾼다.

```python
"""Run the pinned llama-bench matrix for M0. Raw throughput only; see feasibility.py for estimates."""
import argparse
import json
import subprocess
import time
from datetime import datetime
from pathlib import Path

from bench_env import (competing_processes, hide_paths, keep_awake, power_mode, power_status,
                       unique_path, utc_now)
from feasibility import estimate_from_records, render_markdown
from llama_bench_results import normalize, parse_jsonl, select_best

ROOT = Path(__file__).resolve().parents[1]
BLOCKING_PROCESSES = {'llama-server.exe', 'llama-bench.exe', 'llama-cli.exe',
                      'whisper-cli.exe', 'whisper-server.exe'}
JOB_TIMEOUT_SECONDS = 3 * 60 * 60


def screen_jobs():
    jobs = [{'name': f'cpu-t{threads}', 'runtime': 'cpu', 'allow_failure': False,
             'args': ['-ngl', '0', '-t', str(threads), '-fa', 'auto',
                      '-p', '512', '-n', '128', '-d', '0']}
            for threads in (4, 8)]
    for threads in (4, 8):
        for fa in ('off', 'on'):
            jobs.append({'name': f'vulkan-t{threads}-fa-{fa}', 'runtime': 'vulkan',
                         'allow_failure': False,
                         'args': ['-ngl', '99', '-t', str(threads), '-fa', fa, '-ub', '512',
                                  '-p', '512', '-n', '128', '-d', '0,2048']})
    return jobs


def depth_jobs(selected):
    common = ['-ngl', '99', '-t', str(selected['threads']), '-fa', selected['flash_attn'],
              '-p', '512', '-n', '128']
    return [
        {'name': 'vulkan-depth-ub128', 'runtime': 'vulkan', 'allow_failure': False,
         'args': common + ['-ub', '128', '-d', '0,2048,8192']},
        # Known risk: ubatch 512 at 8,192 context lost the Vulkan device in the server test.
        {'name': 'vulkan-depth-ub512', 'runtime': 'vulkan', 'allow_failure': True,
         'args': common + ['-ub', '512', '-d', '8192']},
    ]


def command(job, model_path, runtime_root, repetitions):
    return [str(runtime_root / job['runtime'] / 'llama-bench.exe'), '-m', str(model_path),
            '-r', str(repetitions), '-o', 'jsonl', '--progress', *job['args']]


def tail(path, lines=5):
    """Last log lines with local paths hidden, safe to commit in summaries."""
    text = path.read_text('utf-8', errors='replace') if path.exists() else ''
    return hide_paths('\n'.join(text.splitlines()[-lines:]))


def run_job(job, cmd, out_dir, timeout):
    log_path = out_dir / f"{job['name']}.log"
    started = time.perf_counter()
    with log_path.open('w', encoding='utf-8', errors='replace') as log:
        try:
            completed = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=log, text=True,
                                       encoding='utf-8', errors='replace', timeout=timeout,
                                       creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
            stdout, returncode = completed.stdout, completed.returncode
            status = 'completed' if returncode == 0 else 'failed'
        except subprocess.TimeoutExpired as error:
            stdout, returncode, status = error.output or '', None, 'timeout'
            if isinstance(stdout, bytes):
                stdout = stdout.decode('utf-8', errors='replace')
    (out_dir / f"{job['name']}.jsonl").write_text(stdout, 'utf-8')
    raw, malformed = parse_jsonl(stdout)
    records = [normalize(r, job['runtime']) for r in raw]
    result = {'name': job['name'], 'runtime': job['runtime'], 'args': job['args'],
              'allow_failure': job['allow_failure'], 'status': status,
              'returncode': returncode, 'seconds': round(time.perf_counter() - started, 1),
              'records': len(records), 'malformed_lines': malformed,
              'error_tail': None if status == 'completed' else tail(log_path)}
    return result, records


def overall_status(jobs):
    failed = [j for j in jobs if j['status'] != 'completed']
    if not failed:
        return 'completed'
    if all(j['allow_failure'] for j in failed):
        return 'completed_with_expected_failures'
    return 'failed'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', choices=['screen', 'depth', 'all'], default='all')
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--threads', type=int, help='required with --stage depth')
    parser.add_argument('--flash-attn', choices=['on', 'off'], help='required with --stage depth')
    parser.add_argument('--allow-battery', action='store_true',
                        help='record results on battery power; they are not comparable')
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error('--repetitions must be positive')
    if args.stage == 'depth' and (args.threads is None or args.flash_attn is None):
        parser.error('--stage depth requires --threads and --flash-attn from a screen summary')

    model = json.loads((ROOT / 'config/llm-model.json').read_text('utf-8'))
    runtime = json.loads((ROOT / 'config/llama-runtime.json').read_text('utf-8'))
    model_path = ROOT / 'models' / model['artifact']['filename']
    if not model_path.exists() or model_path.stat().st_size != model['artifact']['size_bytes']:
        raise SystemExit('Model missing or wrong size; run scripts/prepare_llm.py first')
    runtime_root = ROOT / 'runtimes' / runtime['tag']
    if running := competing_processes(BLOCKING_PROCESSES):
        raise SystemExit(f"Stop other inference processes first: {', '.join(running)}")
    power = power_status()
    if power['ac_power'] is not True and not args.allow_battery:
        raise SystemExit(f'Connect AC power before benchmarking (power: {power})')
    mode = power_mode()
    if mode['ac_mode'] != 'best_performance':
        print(f"Warning: targets are judged in best_performance mode; current: {mode['ac_mode']}",
              flush=True)

    out_dir = ROOT / 'artifacts' / f'llama-bench-{time.time_ns()}'
    out_dir.mkdir(parents=True)
    summary = {'schema_version': 1, 'kind': 'llama-bench-throughput',
               'scope': 'raw llama.cpp throughput on synthetic tokens; not app latency or quality',
               'runtime_tag': runtime['tag'], 'model_id': model['model_id'],
               'model_file': model['artifact']['filename'], 'stage': args.stage,
               'repetitions': args.repetitions, 'started_at': utc_now(),
               'power_at_start': power, 'power_mode_at_start': mode, 'selected': None,
               'jobs': [], 'records': []}

    def save():
        (out_dir / 'summary.json').write_text(
            json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')

    def run_all(jobs):
        for job in jobs:
            print(f"[{utc_now()}] {job['name']} ...", flush=True)
            cmd = command(job, model_path, runtime_root, args.repetitions)
            result, records = run_job(job, cmd, out_dir, JOB_TIMEOUT_SECONDS)
            summary['jobs'].append(result)
            summary['records'].extend(records)
            save()
            print(f"[{utc_now()}] {job['name']}: {result['status']} ({result['seconds']}s, "
                  f"{result['records']} records)", flush=True)

    with keep_awake():
        if args.stage in ('screen', 'all'):
            run_all(screen_jobs())
            summary['selected'] = select_best(summary['records'])
        else:
            summary['selected'] = {'threads': args.threads, 'flash_attn': args.flash_attn}
        if args.stage in ('depth', 'all'):
            run_all(depth_jobs(summary['selected']))
            try:
                summary['estimate'] = estimate_from_records(summary['records'],
                                                            summary['selected'])
            except ValueError as error:
                summary['estimate'] = {'error': str(error)}
    summary['power_at_end'] = power_status()
    summary['power_mode_at_end'] = power_mode()
    summary['finished_at'] = utc_now()
    summary['status'] = overall_status(summary['jobs'])
    save()
    stamp = datetime.now().strftime('%Y-%m-%d')
    result_path = unique_path(ROOT / 'evaluation/results' / f'{stamp}-llama-bench-{args.stage}.json')
    result_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')
    print(f'Selected: {summary["selected"]}; status: {summary["status"]}', flush=True)
    estimate = summary.get('estimate', {})
    if 'verdict' in estimate:
        (out_dir / 'estimate.md').write_text(
            render_markdown(summary['records'], estimate) + '\n', 'utf-8')
        print(f"Verdict (raw throughput estimate): {estimate['verdict']}", flush=True)
    elif 'error' in estimate:
        print(f"Estimate unavailable: {estimate['error']}", flush=True)
    print(f'Summary: {result_path}\nLogs: {out_dir}', flush=True)
    if summary['status'] == 'failed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
```

`tests/test_run_llama_bench.py` 전체를 다음으로 바꾼다. 공통 모듈로 옮긴 프로세스·전원·절전·경로 테스트는 `tests/test_bench_env.py`가 맡는다.

```python
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
```

- [ ] **Step 6: 전체 테스트와 인자 검증 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 42 tests`, `OK`

Run: `python scripts/run_llama_bench.py --stage depth`
Expected: 종료 코드 2, `error: --stage depth requires --threads and --flash-attn from a screen summary`

```bash
git add scripts/bench_env.py tests/test_bench_env.py scripts/run_llama_bench.py tests/test_run_llama_bench.py
git commit -m "refactor: share benchmark environment checks and record power mode" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: STT 자산 준비

**Files:**
- Modify: `scripts/prepare_llm.py` (`safe_extract_zip` 분리)
- Modify: `tests/test_prepare_llm.py` (zip 테스트 2개 추가)
- Create: `config/stt-runtime.json`, `config/stt-models.json`, `config/stt-eval-data.json`
- Create: `scripts/prepare_stt.py`
- Test: `tests/test_prepare_stt.py`

**Interfaces:**
- Consumes: `prepare_llm.download(url, target, size=None, digest=None)`, `prepare_llm.download_model(url, target, size, digest)`, `prepare_llm.verify(path, size, digest) -> bool`.
- Produces:
  - `prepare_llm.safe_extract_zip(archive: Path, destination: Path) -> None`: 경로 이탈 시 `RuntimeError('Unsafe archive path')`
  - `prepare_stt.load_config(name) -> dict`, `runtime_jobs(runtime) -> list[tuple]`, `model_jobs(models, selected=None) -> list[tuple]`, `data_jobs(data) -> list[tuple]`, `fetcher_for(size) -> callable`
  - 설정 키: `stt-models.json`의 `models[].id`·`filename`·`size_bytes`·`sha256`, `screen_model`. `stt-runtime.json`의 `tag`·`release_url`·`assets[].build`(`cpu`·`blas`). `stt-eval-data.json`의 `dataset`·`config`·`revision`·`files[].path`
  - 설치 위치: `runtimes/whisper-b5130/<build>/Release/whisper-cli.exe`, `models/whisper/<filename>`, `downloads/fleurs/ko_kr/test.tsv`, `downloads/fleurs/ko_kr/test.tar.gz`

- [ ] **Step 1: 실패하는 zip 해제 테스트 작성**

`tests/test_prepare_llm.py`의 `from unittest.mock import patch` 다음 줄에 `import zipfile`을 추가하고, `test_ignored_range_is_rejected` 앞에 다음 두 테스트를 추가한다.

```python
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
```

Run: `python -m unittest discover -s tests -p test_prepare_llm.py -v`
Expected: FAIL 2개 — `AttributeError: module 'prepare_llm' has no attribute 'safe_extract_zip'`

- [ ] **Step 2: `safe_extract_zip` 분리**

`scripts/prepare_llm.py`의 `def main():`부터 `download_model(` 줄 직전까지를 다음으로 바꾼다.

```python
def safe_extract_zip(archive, destination):
    """Extract only when every member stays inside the destination directory."""
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        for member in zipped.infolist():
            if not (destination / member.filename).resolve().is_relative_to(destination.resolve()):
                raise RuntimeError('Unsafe archive path')
        zipped.extractall(destination)


def main():
    model = json.loads((ROOT / 'config/llm-model.json').read_text('utf-8'))['artifact']
    runtime = json.loads((ROOT / 'config/llama-runtime.json').read_text('utf-8'))
    for asset in runtime['assets']:
        archive = ROOT / 'downloads' / asset['filename']
        url = f"https://github.com/ggml-org/llama.cpp/releases/download/{runtime['tag']}/{asset['filename']}"
        download(url, archive, asset['size_bytes'], asset['sha256'])
        safe_extract_zip(archive, ROOT / 'runtimes' / runtime['tag'] / asset['backend'])
```

Run: `python -m unittest discover -s tests -p test_prepare_llm.py -v`
Expected: PASS — `Ran 5 tests`, `OK`

- [ ] **Step 3: 실패하는 준비 도구 테스트 작성**

`tests/test_prepare_stt.py`:

```python
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
```

Run: `python -m unittest discover -s tests -p test_prepare_stt.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'prepare_stt'`

- [ ] **Step 4: 설정 파일 작성**

`config/stt-runtime.json`:

```json
{
  "engine": "whisper.cpp",
  "tag": "b5130",
  "commit": "927cfce34f31707e17f2bff35c349632fb9e2c3a",
  "release_url": "https://github.com/ggml-org/whisper.cpp/releases/download/b5130",
  "license": "MIT",
  "notes": "Official Windows x64 builds only; no Vulkan build is published. The blas build bundles libopenblas.dll, whose license must be confirmed before redistribution.",
  "assets": [
    {
      "build": "cpu",
      "filename": "whisper-bin-x64.zip",
      "size_bytes": 8573270,
      "sha256": "f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c"
    },
    {
      "build": "blas",
      "filename": "whisper-blas-bin-x64.zip",
      "size_bytes": 21360234,
      "sha256": "55c06d09e8b9b6cfb2b0b47ddedc71803054f0e48be1f41848b3141c06c703a9"
    }
  ]
}
```

`config/stt-models.json`:

```json
{
  "repository": "ggerganov/whisper.cpp",
  "revision": "5359861c739e955e79d9a303bcbc70fb988958b1",
  "license": "MIT",
  "screen_model": "large-v3-turbo-q5_0",
  "models": [
    {
      "id": "small-q5_1",
      "filename": "ggml-small-q5_1.bin",
      "size_bytes": 190085487,
      "sha256": "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"
    },
    {
      "id": "medium-q5_0",
      "filename": "ggml-medium-q5_0.bin",
      "size_bytes": 539212467,
      "sha256": "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f"
    },
    {
      "id": "large-v3-turbo-q5_0",
      "filename": "ggml-large-v3-turbo-q5_0.bin",
      "size_bytes": 574041195,
      "sha256": "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
    },
    {
      "id": "large-v3-turbo",
      "filename": "ggml-large-v3-turbo.bin",
      "size_bytes": 1624555275,
      "sha256": "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"
    },
    {
      "id": "large-v3-q5_0",
      "filename": "ggml-large-v3-q5_0.bin",
      "size_bytes": 1081140203,
      "sha256": "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1"
    },
    {
      "id": "large-v3",
      "filename": "ggml-large-v3.bin",
      "size_bytes": 3095033483,
      "sha256": "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2"
    }
  ]
}
```

`config/stt-eval-data.json`:

```json
{
  "dataset": "google/fleurs",
  "config": "ko_kr",
  "split": "test",
  "revision": "70bb2e84b976b7e960aa89f1c648e09c59f894dd",
  "license": "CC-BY-4.0",
  "attribution": "FLEURS: Few-shot Learning Evaluation of Universal Representations of Speech (Conneau et al., 2022), Google, CC BY 4.0",
  "files": [
    {
      "path": "data/ko_kr/test.tsv",
      "size_bytes": 211844,
      "sha256": "cf2f7c8765f6203e3c46ef620d5e936d3f331b6e8865455557777dd1347517f5"
    },
    {
      "path": "data/ko_kr/audio/test.tar.gz",
      "size_bytes": 214425558,
      "sha256": "3489e529f2aad18d3357b746c5f955941d258b79dc60d8a102d5bced2a223184"
    }
  ]
}
```

- [ ] **Step 5: 준비 도구 구현**

`scripts/prepare_stt.py`:

```python
"""Download pinned STT benchmark assets: whisper.cpp builds, models, FLEURS Korean test."""
import argparse
import json
from pathlib import Path

from prepare_llm import download, download_model, safe_extract_zip

ROOT = Path(__file__).resolve().parents[1]
RANGED_THRESHOLD_BYTES = 64 * 1024 * 1024


def load_config(name):
    return json.loads((ROOT / 'config' / name).read_text('utf-8'))


def runtime_jobs(runtime):
    """(url, archive, size, sha256, extract_dir) for each pinned whisper.cpp build."""
    return [(f"{runtime['release_url']}/{asset['filename']}",
             ROOT / 'downloads/whisper.cpp' / runtime['tag'] / asset['filename'],
             asset['size_bytes'], asset['sha256'],
             ROOT / 'runtimes' / f"whisper-{runtime['tag']}" / asset['build'])
            for asset in runtime['assets']]


def model_jobs(models, selected=None):
    base = f"https://huggingface.co/{models['repository']}/resolve/{models['revision']}"
    return [(f"{base}/{model['filename']}", ROOT / 'models/whisper' / model['filename'],
             model['size_bytes'], model['sha256'])
            for model in models['models'] if not selected or model['id'] in selected]


def data_jobs(data):
    base = f"https://huggingface.co/datasets/{data['dataset']}/resolve/{data['revision']}"
    return [(f"{base}/{item['path']}",
             ROOT / 'downloads/fleurs' / data['config'] / Path(item['path']).name,
             item['size_bytes'], item['sha256'])
            for item in data['files']]


def fetcher_for(size):
    """Large files use resumable byte ranges; small ones a single stream."""
    return download_model if size > RANGED_THRESHOLD_BYTES else download


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--models', nargs='*', help='model ids to fetch (default: all)')
    args = parser.parse_args()
    models = load_config('stt-models.json')
    unknown = set(args.models or []) - {model['id'] for model in models['models']}
    if unknown:
        parser.error(f"Unknown model ids: {', '.join(sorted(unknown))}")
    for url, archive, size, digest, destination in runtime_jobs(load_config('stt-runtime.json')):
        download(url, archive, size, digest)
        safe_extract_zip(archive, destination)
    for url, target, size, digest in data_jobs(load_config('stt-eval-data.json')):
        fetcher_for(size)(url, target, size, digest)
    for url, target, size, digest in model_jobs(models, args.models):
        fetcher_for(size)(url, target, size, digest)


if __name__ == '__main__':
    main()
```

- [ ] **Step 6: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_prepare_stt.py -v`
Expected: PASS — `Ran 5 tests`, `OK`

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 49 tests`, `OK`

- [ ] **Step 7: 자산 내려받기 (약 7.1GB, 노트북 사용 중에도 가능)**

Run: `python scripts/prepare_stt.py` (백그라운드 실행 권장)
Expected: 이미 받은 파일은 `Verified existing ...`, 나머지는 `Installed and SHA-256 verified ...`로 끝나고 종료 코드 0. 연결이 끊기면 같은 명령을 다시 실행한다. 받은 구간은 재사용된다.

실행 중 발견(2026-09-17): 첫 실행에서 8개 연결이 동시에 끊기고(`Incomplete range`), 재실행에서도 일부 연결이 응답 없이 멈췄다. 기존 도구는 끊긴 구간을 처음부터 다시 받아 진행분을 버리므로, `prepare_llm.download_model`이 구간 파일에 이미 받은 바이트 다음부터 이어 받도록 고쳤다(테스트 `test_interrupted_chunk_resumes_after_cached_bytes` 추가, 전체 81개). 이 수정은 Task 5를 마친 뒤 반영했으므로 Task 3~5의 전체 테스트 수(59·68·80)는 수정 전 기준이고, Task 6에서는 81개다.

`tests/test_prepare_llm.py`의 zip 테스트 앞에 추가한 테스트(수정 전 코드에서는 `Server did not honor exact byte range`로 실패한다):

```python
    def test_interrupted_chunk_resumes_after_cached_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'model.gguf'
            (Path(directory) / 'model.gguf.parts').mkdir()
            (Path(directory) / 'model.gguf.parts/0').write_bytes(b'he')
            digest = hashlib.sha256(b'hello').hexdigest()
            rest = Response(b'llo', content_range='bytes 2-4/5')
            with patch.object(MODULE.urllib.request, 'urlopen', return_value=rest) as fetch:
                MODULE.download_model('https://example.invalid/model', target, 5, digest)
            self.assertEqual(fetch.call_args.args[0].get_header('Range'), 'bytes=2-4')
            self.assertEqual(target.read_bytes(), b'hello')
```

`scripts/prepare_llm.py`의 `download_model` 안 `fetch` 함수 전체:

```python
    def fetch(index):
        start = index * chunk_size
        end = min(start + chunk_size, size) - 1
        part = parts / str(index)
        have = part.stat().st_size if part.exists() else 0
        if have > end - start + 1:
            part.unlink()
            have = 0
        if have == end - start + 1:
            return part
        # Resume after the bytes already cached; distinct cache keys per offset prevent
        # intermediary caches serializing byte ranges.
        first = start + have
        range_url = url + ('&' if '?' in url else '?') + f'download=true&part={index}&from={have}'
        request = urllib.request.Request(range_url, headers={
            'User-Agent': 'Just-a-Click-M0', 'Range': f'bytes={first}-{end}'})
        with urllib.request.urlopen(request, timeout=90) as response:
            expected = f'bytes {first}-{end}/{size}'
            if response.status != 206 or response.headers.get('Content-Range') != expected:
                raise RuntimeError('Server did not honor exact byte range')
            with part.open('ab') as output:
                shutil.copyfileobj(response, output, 1024 * 1024)
        if part.stat().st_size != end - start + 1:
            raise RuntimeError('Incomplete range')
        print(f'Model chunk {index + 1} complete', flush=True)
        return part
```

Run: `python -c "import json,sys; sys.path.insert(0,'scripts'); from prepare_llm import verify; from pathlib import Path; m=json.load(open('config/stt-models.json',encoding='utf-8')); print(all(verify(Path('models/whisper')/x['filename'], x['size_bytes'], x['sha256']) for x in m['models']))"`
Expected: `True`

- [ ] **Step 8: 커밋**

```bash
git add scripts/prepare_llm.py tests/test_prepare_llm.py config/stt-runtime.json config/stt-models.json config/stt-eval-data.json scripts/prepare_stt.py tests/test_prepare_stt.py
git commit -m "feat: prepare pinned whisper.cpp builds, models and FLEURS data" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: STT 지표

**Files:**
- Create: `scripts/stt_metrics.py`
- Test: `tests/test_stt_metrics.py`

**Interfaces:**
- Consumes: 없음.
- Produces:
  - `normalize(text) -> str`, `hangul_only(text) -> str`, `edit_distance(reference, hypothesis) -> int`
  - `char_errors(reference, hypothesis) -> dict`: 키 `ref_chars`, `hyp_chars`, `errors`, `ref_hangul`, `errors_hangul`
  - `error_rate(errors, total) -> float | None`(소수 4자리), `suspected_anomaly(errors: dict) -> bool`
  - `parse_timings(stderr) -> dict`: 키 `load_ms`, `total_ms`, 선택 `fallbacks`, `decoding`(`threads`·`processors`·`beams`·`best_of`·`language`). 시간 줄이 없으면 `ValueError`
  - `percentile(values, fraction) -> float`: 최근접 순위, 빈 목록이면 `ValueError`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_stt_metrics.py`:

```python
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
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_stt_metrics.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'stt_metrics'`

- [ ] **Step 3: 모듈 구현**

`scripts/stt_metrics.py`:

```python
"""Korean STT metrics: normalization, character error rates, whisper.cpp timing lines."""
import math
import re
import unicodedata

TIMING = re.compile(r'whisper_print_timings:\s+(load|total) time =\s+([0-9.]+) ms')
FALLBACKS = re.compile(r'whisper_print_timings:\s+fallbacks =\s+(\d+) p /\s+(\d+) h')
DECODING = re.compile(r'(\d+) threads, (\d+) processors, (\d+) beams \+ best of (\d+), lang = (\w+)')
HANGUL_SYLLABLE = re.compile('[가-힣]')


def normalize(text):
    """NFKC, lowercase, keep letters and digits only (drops spaces and punctuation)."""
    text = unicodedata.normalize('NFKC', text).lower()
    return ''.join(ch for ch in text if ch.isalnum())


def hangul_only(text):
    return ''.join(HANGUL_SYLLABLE.findall(unicodedata.normalize('NFKC', text)))


def edit_distance(reference, hypothesis):
    previous = list(range(len(hypothesis) + 1))
    for i, ref_char in enumerate(reference, 1):
        current = [i]
        for j, hyp_char in enumerate(hypothesis, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1,
                               previous[j - 1] + (ref_char != hyp_char)))
        previous = current
    return previous[-1]


def char_errors(reference, hypothesis):
    ref, hyp = normalize(reference), normalize(hypothesis)
    ref_hangul, hyp_hangul = hangul_only(reference), hangul_only(hypothesis)
    return {'ref_chars': len(ref), 'hyp_chars': len(hyp), 'errors': edit_distance(ref, hyp),
            'ref_hangul': len(ref_hangul),
            'errors_hangul': edit_distance(ref_hangul, hyp_hangul)}


def error_rate(errors, total):
    return round(errors / total, 4) if total else None


def suspected_anomaly(errors):
    """Chunk CER above 50% or output at least twice as long as the reference."""
    return (errors['ref_chars'] > 0 and errors['errors'] / errors['ref_chars'] > 0.5) \
        or errors['hyp_chars'] >= 2 * max(errors['ref_chars'], 1)


def parse_timings(stderr):
    values = {name: float(ms) for name, ms in TIMING.findall(stderr)}
    if set(values) != {'load', 'total'}:
        raise ValueError('whisper timing lines are missing')
    result = {'load_ms': values['load'], 'total_ms': values['total']}
    if match := FALLBACKS.search(stderr):
        result['fallbacks'] = int(match[1]) + int(match[2])
    if match := DECODING.search(stderr):
        result['decoding'] = {'threads': int(match[1]), 'processors': int(match[2]),
                              'beams': int(match[3]), 'best_of': int(match[4]),
                              'language': match[5]}
    return result


def percentile(values, fraction):
    """Nearest-rank percentile: the ceil(fraction * n)-th smallest value."""
    ordered = sorted(values)
    if not ordered:
        raise ValueError('No values')
    return ordered[max(1, math.ceil(fraction * len(ordered))) - 1]
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_stt_metrics.py -v`
Expected: PASS — `Ran 10 tests`, `OK`

- [ ] **Step 5: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 59 tests`, `OK`

```bash
git add scripts/stt_metrics.py tests/test_stt_metrics.py
git commit -m "feat: add Korean character error rates and whisper timing parsing" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: FLEURS 조각 생성

**Files:**
- Create: `scripts/stt_data.py`
- Test: `tests/test_stt_data.py`
- Create: `evaluation/fixtures/stt-fleurs-ko-v1.json` (실행 결과)

**Interfaces:**
- Consumes: `prepare_llm.verify`, Task 2의 `config/stt-eval-data.json`과 `downloads/fleurs/ko_kr/` 파일.
- Produces:
  - 상수 `FIXTURE_ID = 'stt-fleurs-ko-v1'`, `SAMPLE_RATE = 16000`, `CHUNK_COUNT = 60`
  - `chunk_dir(fixture_id=FIXTURE_ID) -> Path` (`artifacts/stt-chunks/stt-fleurs-ko-v1`)
  - `parse_tsv(text) -> list[dict]`(키 `sentence_id`, `file_name`, `raw_transcription`, `transcription`, `samples`, `gender`), `select_utterances(rows) -> list[dict]`
  - `plan_chunks(utterances, limit_seconds=30.0, gap_seconds=0.5, count=60, rate=16000) -> list[dict]`
  - 조각 키: `chunk_id`(`fleurs-ko-001`…), `samples`, `seconds`, `utterances[]`(`sentence_id`·`file_name`·`samples`), `reference`
  - `read_wav_samples(data) -> array('f')`, `to_pcm16(samples) -> array('h')`, `write_wav(path, pcm)`
  - `build(tsv_path, tar_path, out_dir, source, count=CHUNK_COUNT) -> dict`(조각 목록 파일 내용), `verify_chunks(fixture, directory) -> list[str]`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_stt_data.py`:

```python
import array
import io
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest
import wave

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import stt_data as D  # noqa: E402


def float_wav(values, rate=16000, channels=1):
    payload = array.array('f', values).tobytes()
    fmt = struct.pack('<HHIIHH', 3, channels, rate, rate * 4 * channels, 4 * channels, 32)
    body = (b'WAVE' + b'fmt ' + struct.pack('<I', len(fmt)) + fmt
            + b'fact' + struct.pack('<I', 4) + struct.pack('<I', len(values))
            + b'data' + struct.pack('<I', len(payload)) + payload)
    return b'RIFF' + struct.pack('<I', len(body)) + body


def pcm_wav(values):
    buffer = io.BytesIO()
    with wave.open(buffer, 'wb') as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(16000)
        audio.writeframes(array.array('h', values).tobytes())
    return buffer.getvalue()


def row(sentence_id, name, samples, text):
    return f'{sentence_id}\t{name}\tRAW {text}\t{text}\tx | y\t{samples}\tMALE'


class TsvTests(unittest.TestCase):
    def test_parse_and_select_first_recording_per_sentence(self):
        text = '\n'.join([row(10, 'b.wav', 5, '열'), row(9, 'z.wav', 5, '아홉'),
                          row(10, 'a.wav', 6, '십')]) + '\n'
        chosen = D.select_utterances(D.parse_tsv(text))
        self.assertEqual([(u['sentence_id'], u['file_name']) for u in chosen],
                         [(9, 'z.wav'), (10, 'a.wav')])
        self.assertEqual(chosen[1]['transcription'], '십')

    def test_malformed_row_rejected(self):
        with self.assertRaises(ValueError):
            D.parse_tsv('1\ta.wav\tonly three')


class ChunkPlanTests(unittest.TestCase):
    def utterance(self, sentence_id, samples):
        return {'sentence_id': sentence_id, 'file_name': f'{sentence_id}.wav',
                'samples': samples, 'transcription': f't{sentence_id}'}

    def test_packs_within_limit_with_gaps_and_skips_long_items(self):
        items = [self.utterance(i, n) for i, n in enumerate([100, 100, 100, 350, 50, 60], 1)]
        chunks = D.plan_chunks(items, limit_seconds=30, gap_seconds=0.5, count=5, rate=10)
        self.assertEqual([[u['sentence_id'] for u in c['utterances']] for c in chunks],
                         [[1, 2], [3, 5, 6]])
        self.assertEqual([c['samples'] for c in chunks], [205, 220])
        self.assertEqual(chunks[1]['reference'], 't3 t5 t6')
        self.assertEqual((chunks[0]['chunk_id'], chunks[0]['seconds']), ('fleurs-ko-001', 20.5))

    def test_stops_at_requested_count(self):
        items = [self.utterance(i, 200) for i in range(1, 6)]
        chunks = D.plan_chunks(items, limit_seconds=30, gap_seconds=0.5, count=2, rate=10)
        self.assertEqual(len(chunks), 2)
        self.assertEqual(chunks[-1]['utterances'][0]['sentence_id'], 2)


class WavTests(unittest.TestCase):
    def test_reads_float_and_pcm16(self):
        self.assertEqual(list(D.read_wav_samples(float_wav([0.5, -0.25]))), [0.5, -0.25])
        self.assertEqual(list(D.read_wav_samples(pcm_wav([16384, -32768]))), [0.5, -1.0])

    def test_rejects_unexpected_audio(self):
        with self.assertRaises(ValueError):
            D.read_wav_samples(float_wav([0.0, 0.0], channels=2))
        with self.assertRaises(ValueError):
            D.read_wav_samples(float_wav([0.0], rate=48000))
        with self.assertRaises(ValueError):
            D.read_wav_samples(b'not a wav file')

    def test_pcm16_conversion_clips(self):
        self.assertEqual(list(D.to_pcm16([0.5, 1.5, -2.0, 0.0])), [16384, 32767, -32768, 0])


class BuildTests(unittest.TestCase):
    def test_build_writes_verifiable_chunks(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            tsv = base / 'test.tsv'
            tsv.write_text(row(2, 'b.wav', 1600, '둘') + '\n' + row(1, 'a.wav', 1600, '하나') + '\n',
                           'utf-8')
            tar_path = base / 'test.tar.gz'
            with tarfile.open(tar_path, 'w:gz') as tar:
                for name, value in (('b.wav', -0.5), ('a.wav', 0.5)):
                    data = float_wav([value] * 1600)
                    info = tarfile.TarInfo(f'test/{name}')
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
            out = base / 'chunks'
            fixture = D.build(tsv, tar_path, out, {'dataset': 'google/fleurs'}, count=1)
            chunk = fixture['chunks'][0]
            self.assertEqual(chunk['samples'], 1600 + 8000 + 1600)
            self.assertEqual(chunk['reference'], '하나 둘')
            with wave.open(str(out / 'fleurs-ko-001.wav')) as audio:
                frames = array.array('h', audio.readframes(audio.getnframes()))
            self.assertEqual((frames[0], frames[1600], frames[-1]), (16384, 0, -16384))
            self.assertEqual(D.verify_chunks(fixture, out), [])
            (out / 'fleurs-ko-001.wav').unlink()
            self.assertEqual(D.verify_chunks(fixture, out), ['missing fleurs-ko-001'])

    def test_build_rejects_sample_count_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            tsv = base / 'test.tsv'
            tsv.write_text(row(1, 'a.wav', 999, '하나') + '\n', 'utf-8')
            tar_path = base / 'test.tar.gz'
            with tarfile.open(tar_path, 'w:gz') as tar:
                data = float_wav([0.1] * 10)
                info = tarfile.TarInfo('test/a.wav')
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
            with self.assertRaisesRegex(ValueError, 'Sample count mismatch'):
                D.build(tsv, tar_path, base / 'out', {}, count=1)


if __name__ == '__main__':
    unittest.main()
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_stt_data.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'stt_data'`

- [ ] **Step 3: 모듈 구현**

`scripts/stt_data.py`:

```python
"""Build the FLEURS Korean 30-second chunk set. Public CC BY 4.0 read speech, not lecture audio."""
import array
import json
from pathlib import Path, PurePosixPath
import struct
import tarfile
import wave

from prepare_llm import verify

ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ID = 'stt-fleurs-ko-v1'
SAMPLE_RATE = 16000
CHUNK_LIMIT_SECONDS = 30.0
GAP_SECONDS = 0.5
CHUNK_COUNT = 60
SELECTION = {'recording': 'one per sentence id, lexicographically first file name',
             'order': 'sentence id ascending', 'gap_seconds': GAP_SECONDS,
             'max_chunk_seconds': CHUNK_LIMIT_SECONDS, 'chunks': CHUNK_COUNT,
             'reference': 'FLEURS normalized transcription (4th column) joined by spaces'}


def chunk_dir(fixture_id=FIXTURE_ID):
    return ROOT / 'artifacts/stt-chunks' / fixture_id


def parse_tsv(text):
    rows = []
    for line in text.splitlines():
        if not line.strip():
            continue
        fields = line.split('\t')
        if len(fields) != 7:
            raise ValueError('FLEURS rows must have 7 tab-separated fields')
        sentence_id, file_name, raw, normalized, _, samples, gender = fields
        rows.append({'sentence_id': int(sentence_id), 'file_name': file_name,
                     'raw_transcription': raw, 'transcription': normalized,
                     'samples': int(samples), 'gender': gender})
    return rows


def select_utterances(rows):
    """One recording per sentence (lexicographically first file name), ordered by sentence id."""
    chosen = {}
    for row in rows:
        best = chosen.get(row['sentence_id'])
        if best is None or row['file_name'] < best['file_name']:
            chosen[row['sentence_id']] = row
    return [chosen[key] for key in sorted(chosen)]


def chunk_record(index, group, gap, rate):
    samples = sum(u['samples'] for u in group) + gap * (len(group) - 1)
    return {'chunk_id': f'fleurs-ko-{index:03d}', 'samples': samples,
            'seconds': round(samples / rate, 3),
            'utterances': [{'sentence_id': u['sentence_id'], 'file_name': u['file_name'],
                            'samples': u['samples']} for u in group],
            'reference': ' '.join(u['transcription'] for u in group)}


def plan_chunks(utterances, limit_seconds=CHUNK_LIMIT_SECONDS, gap_seconds=GAP_SECONDS,
                count=CHUNK_COUNT, rate=SAMPLE_RATE):
    limit, gap = round(limit_seconds * rate), round(gap_seconds * rate)
    groups, current, length = [], [], 0
    for utterance in utterances:
        if utterance['samples'] > limit:
            continue
        added = utterance['samples'] + (gap if current else 0)
        if current and length + added > limit:
            groups.append(current)
            if len(groups) == count:
                break
            current, length, added = [], 0, utterance['samples']
        current.append(utterance)
        length += added
    else:
        if current:
            groups.append(current)
    return [chunk_record(i, group, gap, rate) for i, group in enumerate(groups[:count], 1)]


def read_wav_samples(data):
    """Mono 16 kHz samples in [-1, 1] from PCM16 or IEEE float32 RIFF bytes (little-endian)."""
    if data[:4] != b'RIFF' or data[8:12] != b'WAVE':
        raise ValueError('Not a RIFF/WAVE file')
    position, fmt, payload = 12, None, None
    while position + 8 <= len(data):
        chunk_id = data[position:position + 4]
        size = struct.unpack('<I', data[position + 4:position + 8])[0]
        body = data[position + 8:position + 8 + size]
        if chunk_id == b'fmt ':
            fmt = struct.unpack('<HHIIHH', body[:16])
        elif chunk_id == b'data':
            payload = body
        position += 8 + size + (size & 1)
    if fmt is None or payload is None:
        raise ValueError('WAV is missing a fmt or data chunk')
    tag, channels, rate, _, _, bits = fmt
    if channels != 1 or rate != SAMPLE_RATE:
        raise ValueError('Expected mono 16 kHz audio')
    if tag == 3 and bits == 32:
        samples = array.array('f')
        samples.frombytes(payload[:len(payload) // 4 * 4])
        return samples
    if tag == 1 and bits == 16:
        ints = array.array('h')
        ints.frombytes(payload[:len(payload) // 2 * 2])
        return array.array('f', (value / 32768 for value in ints))
    raise ValueError(f'Unsupported WAV encoding: format {tag}, {bits} bits')


def to_pcm16(samples):
    return array.array('h', (max(-32768, min(32767, round(s * 32767))) for s in samples))


def write_wav(path, pcm):
    with wave.open(str(path), 'wb') as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(SAMPLE_RATE)
        audio.writeframes(pcm.tobytes())


def build(tsv_path, tar_path, out_dir, source, count=CHUNK_COUNT):
    chunks = plan_chunks(select_utterances(parse_tsv(tsv_path.read_text('utf-8'))), count=count)
    if len(chunks) != count:
        raise ValueError(f'Only {len(chunks)} of {count} chunks could be built')
    needed = {u['file_name']: u['samples'] for chunk in chunks for u in chunk['utterances']}
    audio = {}
    with tarfile.open(tar_path) as tar:
        # One sequential pass: seeking backwards in a gzip tar decompresses from the start.
        for member in tar:
            name = PurePosixPath(member.name).name
            if member.isfile() and name in needed:
                samples = read_wav_samples(tar.extractfile(member).read())
                if len(samples) != needed[name]:
                    raise ValueError(f'Sample count mismatch for {name}')
                audio[name] = to_pcm16(samples)
    if missing := sorted(set(needed) - set(audio)):
        raise ValueError(f'Audio missing from archive: {missing[:3]}')
    out_dir.mkdir(parents=True, exist_ok=True)
    gap = array.array('h', bytes(2 * round(GAP_SECONDS * SAMPLE_RATE)))
    for chunk in chunks:
        pcm = array.array('h')
        for index, utterance in enumerate(chunk['utterances']):
            if index:
                pcm.extend(gap)
            pcm.extend(audio[utterance['file_name']])
        write_wav(out_dir / f"{chunk['chunk_id']}.wav", pcm)
    return {'schema_version': 1, 'fixture_id': FIXTURE_ID, 'source': source,
            'selection': dict(SELECTION, chunks=count), 'sample_rate': SAMPLE_RATE,
            'chunks': chunks}


def verify_chunks(fixture, directory):
    problems = []
    for chunk in fixture['chunks']:
        path = directory / f"{chunk['chunk_id']}.wav"
        if not path.exists():
            problems.append(f"missing {chunk['chunk_id']}")
            continue
        with wave.open(str(path)) as audio:
            shape = (audio.getnchannels(), audio.getframerate(), audio.getsampwidth(),
                     audio.getnframes())
        if shape != (1, SAMPLE_RATE, 2, chunk['samples']):
            problems.append(f"invalid {chunk['chunk_id']}")
    return problems


def main():
    source = json.loads((ROOT / 'config/stt-eval-data.json').read_text('utf-8'))
    folder = ROOT / 'downloads/fleurs' / source['config']
    for item in source['files']:
        if not verify(folder / PurePosixPath(item['path']).name, item['size_bytes'], item['sha256']):
            raise SystemExit('FLEURS files missing or corrupt; run scripts/prepare_stt.py first')
    fixture = build(folder / 'test.tsv', folder / 'test.tar.gz', chunk_dir(), source)
    path = ROOT / 'evaluation/fixtures' / f'{FIXTURE_ID}.json'
    path.write_text(json.dumps(fixture, ensure_ascii=False, indent=2) + '\n', 'utf-8')
    total = sum(chunk['seconds'] for chunk in fixture['chunks'])
    print(f"Wrote {len(fixture['chunks'])} chunks ({total:.1f}s) and {path.name}", flush=True)


if __name__ == '__main__':
    main()
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_stt_data.py -v`
Expected: PASS — `Ran 9 tests`, `OK`

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 68 tests`, `OK`

- [ ] **Step 5: 실제 조각과 조각 목록 생성**

Run: `python scripts/stt_data.py`
Expected: `Wrote 60 chunks (1406.0s) and stt-fleurs-ko-v1.json`(약 10초)

Run: `python -c "import json,sys; sys.path.insert(0,'scripts'); import stt_data as D; f=json.load(open('evaluation/fixtures/stt-fleurs-ko-v1.json',encoding='utf-8')); print(len(f['chunks']), D.verify_chunks(f, D.chunk_dir()), f['source']['license'])"`
Expected: `60 [] CC-BY-4.0`

- [ ] **Step 6: 커밋**

`artifacts/` 아래 조각 WAV는 Git에서 제외된다. `git status --short`에 WAV가 보이지 않는지 확인한다.

```bash
git add scripts/stt_data.py tests/test_stt_data.py evaluation/fixtures/stt-fleurs-ko-v1.json
git commit -m "feat: build FLEURS Korean 30-second STT chunks" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: STT 측정 실행기

**Files:**
- Create: `scripts/run_stt_bench.py`
- Test: `tests/test_run_stt_bench.py`
- Modify: `evaluation/README.md` (`## 측정 범위` 바로 앞에 절 추가)

**Interfaces:**
- Consumes: `bench_env`(Task 1), `prepare_llm.verify`, `stt_data.FIXTURE_ID`·`chunk_dir`·`verify_chunks`(Task 4), `stt_metrics`(Task 3), 설정 파일(Task 2).
- Produces:
  - `screen_configs() -> list[dict]`, `cli_command(executable, model_path, wav_path, threads, output_prefix, timestamps=False) -> list[str]`
  - `transcript_text(result) -> str`, `run_chunk(cmd, output_prefix, chunk, timeout=300) -> dict`
  - 조각 기록 키: 공통 `chunk_id`, `seconds`, `status`(`completed`·`failed`·`timeout`), `wall_seconds`. 종료 코드를 받으면 `returncode`. 완료 시 `load_seconds`, `processing_seconds`, `rtf`, `fallbacks`, `decoding`, `anomaly_suspect`, `ref_chars`, `hyp_chars`, `errors`, `ref_hangul`, `errors_hangul`. 실패 시 `error`(경로 가림). `decoding`은 실행기가 꺼내 실행 묶음에 한 번만 기록한다
  - `summarize(records, expected) -> dict`: 키 `expected_chunks`, `completed`, `failures`, `judgeable`, `realtime_ok`. 완료 조각이 있으면 `rtf_median`, `rtf_p95`, `rtf_max`, `rtf_total`, `load_seconds_median`, `cer`, `cer_hangul`, `fallbacks`, `anomaly_suspects`도 기록
  - `select_config(entries) -> dict`(`build`·`threads`, 모든 조각을 마친 조합이 없으면 `ValueError`), `select_model(entries) -> str | None`, `top_models(entries, count=2) -> list[str]`, `render_markdown(summary) -> str`
  - 실행 묶음(entry) 키: `model`, `build`, `threads`, `timestamps`, `decoding`, `records`, `summary`
  - 요약 JSON: `screen`·`models`·`timestamp_check` 실행 묶음 배열, `selected_config`, `selected_model`, `status`(`completed`·`failed`), `criteria`, `diagnostic`, `power_at_start`·`power_at_end`, `power_mode_at_start`·`power_mode_at_end`, `started_at`·`finished_at`
  - 결과 위치: 전체 실행은 `evaluation/results/<날짜>-stt-bench-<stage>.json`, 진단 실행(`--models` 또는 `--chunks`)은 `artifacts/stt-bench-<ns>/`만

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_run_stt_bench.py`:

```python
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import run_stt_bench as S  # noqa: E402

STDERR = ("main: processing 'x.wav' (480000 samples, 30.0 sec), 8 threads, 1 processors, "
          "5 beams + best of 5, lang = ko, task = transcribe, timestamps = 1 ...\n"
          "whisper_print_timings:     load time =   500.00 ms\n"
          "whisper_print_timings:     fallbacks =   0 p /   1 h\n"
          "whisper_print_timings:    total time =  6500.00 ms\n")
CHUNK = {'chunk_id': 'fleurs-ko-001', 'seconds': 20.0, 'reference': '다리 밑 간격'}


def record(chunk_id, rtf, errors=0, ref=10, status='completed', suspect=False):
    return {'chunk_id': chunk_id, 'seconds': 20.0, 'status': status, 'rtf': rtf,
            'processing_seconds': rtf * 20.0, 'load_seconds': 0.5, 'fallbacks': 1,
            'errors': errors, 'ref_chars': ref, 'hyp_chars': ref, 'errors_hangul': errors,
            'ref_hangul': ref, 'anomaly_suspect': suspect}


class CommandTests(unittest.TestCase):
    def test_screen_grid(self):
        self.assertEqual([(c['build'], c['threads']) for c in S.screen_configs()],
                         [('cpu', 4), ('cpu', 8), ('blas', 4), ('blas', 8)])

    def test_cli_command_defaults_to_no_timestamps(self):
        args = (Path('w/whisper-cli.exe'), Path('m.bin'), Path('c.wav'), 8, Path('o/c'))
        base = ['-m', 'm.bin', '-f', 'c.wav', '-l', 'ko', '-t', '8', '-oj', '-of', str(Path('o/c'))]
        self.assertEqual(S.cli_command(*args)[1:], base + ['-nt'])
        self.assertEqual(S.cli_command(*args, timestamps=True)[1:], base)

    def test_transcript_text_joins_segments(self):
        result = {'transcription': [{'text': ' 다리 밑'}, {'text': ' 간격 '}]}
        self.assertEqual(S.transcript_text(result), '다리 밑 간격')


class RunChunkTests(unittest.TestCase):
    def test_completed_chunk_measures_processing_without_load(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / 'fleurs-ko-001'
            Path(f'{prefix}.json').write_text(
                json.dumps({'transcription': [{'text': ' 다리미 간격'}]}), 'utf-8')
            done = subprocess.CompletedProcess(['x'], 0, stdout='', stderr=STDERR)
            with patch.object(S.subprocess, 'run', return_value=done):
                result = S.run_chunk(['x'], prefix, CHUNK)
        self.assertEqual(result['status'], 'completed')
        self.assertEqual((result['load_seconds'], result['processing_seconds']), (0.5, 6.0))
        self.assertEqual(result['rtf'], 0.3)
        self.assertEqual((result['errors'], result['ref_chars'], result['fallbacks']), (1, 5, 1))
        self.assertEqual(result['decoding']['beams'], 5)
        self.assertFalse(result['anomaly_suspect'])

    def test_failed_exit_hides_paths(self):
        done = subprocess.CompletedProcess(['x'], 3, stdout='', stderr=f'error in {S.ROOT}\\m.bin')
        with patch.object(S.subprocess, 'run', return_value=done):
            result = S.run_chunk(['x'], Path('missing'), CHUNK)
        self.assertEqual((result['status'], result['returncode']), ('failed', 3))
        self.assertIn('<repo>\\m.bin', result['error'])
        self.assertNotIn(str(S.ROOT), result['error'])

    def test_missing_output_or_timeout(self):
        done = subprocess.CompletedProcess(['x'], 0, stdout='', stderr=STDERR)
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(S.subprocess, 'run', return_value=done):
            result = S.run_chunk(['x'], Path(directory) / 'none', CHUNK)
        self.assertEqual(result['status'], 'failed')
        with patch.object(S.subprocess, 'run', side_effect=subprocess.TimeoutExpired(['x'], 1)):
            self.assertEqual(S.run_chunk(['x'], Path('p'), CHUNK)['status'], 'timeout')


class SummaryTests(unittest.TestCase):
    def test_realtime_needs_median_p95_and_few_failures(self):
        records = [record(f'c{i}', 0.3, errors=1) for i in range(19)] + [record('c19', 0.9)]
        summary = S.summarize(records, 20)
        self.assertEqual((summary['rtf_median'], summary['rtf_p95'], summary['rtf_max']),
                         (0.3, 0.3, 0.9))
        self.assertEqual((summary['cer'], summary['rtf_total']), (0.095, 0.33))
        self.assertTrue(summary['realtime_ok'])
        slow = [record(f'c{i}', 0.3) for i in range(18)] + [record('a', 0.9), record('b', 0.9)]
        self.assertFalse(S.summarize(slow, 20)['realtime_ok'])
        failed = [record(f'c{i}', 0.3) for i in range(17)]
        self.assertFalse(S.summarize(failed, 20)['judgeable'])
        self.assertFalse(S.summarize([], 5)['realtime_ok'])

    def test_anomaly_suspects_listed(self):
        summary = S.summarize([record('c1', 0.2, suspect=True), record('c2', 0.2)], 2)
        self.assertEqual(summary['anomaly_suspects'], ['c1'])


class SelectionTests(unittest.TestCase):
    def entry(self, model, cer, rtf, ok=True, completed=10, expected=10, judgeable=True):
        return {'model': model, 'build': 'cpu', 'threads': 4, 'timestamps': False,
                'summary': {'cer': cer, 'rtf_median': rtf, 'realtime_ok': ok,
                            'completed': completed, 'expected_chunks': expected,
                            'judgeable': judgeable}}

    def test_select_config_uses_fastest_complete_run(self):
        fast_incomplete = dict(self.entry('m', 0.1, 0.1, completed=9), build='blas', threads=8)
        slow = self.entry('m', 0.1, 0.4)
        faster = dict(self.entry('m', 0.1, 0.3), threads=8)
        self.assertEqual(S.select_config([fast_incomplete, slow, faster]),
                         {'build': 'cpu', 'threads': 8})
        with self.assertRaises(ValueError):
            S.select_config([fast_incomplete])

    def test_select_model_prefers_speed_within_half_point(self):
        entries = [self.entry('large', 0.080, 0.45), self.entry('turbo', 0.084, 0.20),
                   self.entry('small', 0.150, 0.05), self.entry('huge', 0.050, 0.90, ok=False)]
        self.assertEqual(S.select_model(entries), 'turbo')
        entries[1]['summary']['cer'] = 0.086
        self.assertEqual(S.select_model(entries), 'large')
        self.assertIsNone(S.select_model([self.entry('huge', 0.05, 0.9, ok=False)]))

    def test_top_models_ranks_judgeable_by_cer(self):
        entries = [self.entry('a', 0.10, 0.1), self.entry('b', 0.05, 0.9, ok=False),
                   self.entry('c', 0.01, 0.1, judgeable=False), self.entry('d', 0.07, 0.2),
                   self.entry('e', None, 0.2)]
        self.assertEqual(S.top_models(entries), ['b', 'd'])

    def test_render_markdown_lists_runs(self):
        run = dict(self.entry('turbo', 0.084, 0.2), summary=S.summarize([record('c1', 0.2)], 1))
        summary = {'screen': [], 'models': [run],
                   'timestamp_check': [dict(run, timestamps=True)],
                   'selected_config': {'build': 'cpu', 'threads': 4}, 'selected_model': 'turbo'}
        table = S.render_markdown(summary)
        self.assertIn('| models | turbo | cpu | 4 | -nt | 1/1 |', table)
        self.assertIn('| timestamp_check | turbo | cpu | 4 | 사용 | 1/1 |', table)
        self.assertIn('1차 후보 모델(추정): turbo', table)


if __name__ == '__main__':
    unittest.main()
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_run_stt_bench.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'run_stt_bench'`

- [ ] **Step 3: 실행기 구현**

`scripts/run_stt_bench.py`:

```python
"""Run the pinned whisper.cpp CPU benchmark on FLEURS Korean chunks. Estimates, not app latency."""
import argparse
from datetime import datetime
import json
from pathlib import Path
import statistics
import subprocess
import time

from bench_env import (competing_processes, hide_paths, keep_awake, power_mode, power_status,
                       unique_path, utc_now)
from prepare_llm import verify
from stt_data import FIXTURE_ID, chunk_dir, verify_chunks
from stt_metrics import (char_errors, error_rate, parse_timings, percentile,
                         suspected_anomaly)

ROOT = Path(__file__).resolve().parents[1]
BLOCKING_PROCESSES = {'llama-server.exe', 'llama-bench.exe', 'llama-cli.exe',
                      'whisper-cli.exe', 'whisper-server.exe'}
CHUNK_TIMEOUT_SECONDS = 300
SCREEN_CHUNKS = 10
RTF_MEDIAN_LIMIT, RTF_P95_LIMIT = 0.5, 0.8
CER_TIE = 0.005
MAX_FAILURE_RATIO = 0.1


def load_config(name):
    return json.loads((ROOT / 'config' / name).read_text('utf-8'))


def screen_configs():
    return [{'build': build, 'threads': threads} for build in ('cpu', 'blas') for threads in (4, 8)]


def cli_command(executable, model_path, wav_path, threads, output_prefix, timestamps=False):
    """Comparison runs use -nt: timestamp decoding dropped sentences in the smoke test."""
    command = [str(executable), '-m', str(model_path), '-f', str(wav_path), '-l', 'ko',
               '-t', str(threads), '-oj', '-of', str(output_prefix)]
    return command if timestamps else command + ['-nt']


def transcript_text(result):
    return ''.join(segment['text'] for segment in result['transcription']).strip()


def run_chunk(cmd, output_prefix, chunk, timeout=CHUNK_TIMEOUT_SECONDS):
    started = time.perf_counter()
    record = {'chunk_id': chunk['chunk_id'], 'seconds': chunk['seconds']}
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, encoding='utf-8',
                              errors='replace', timeout=timeout,
                              creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    except subprocess.TimeoutExpired:
        return dict(record, status='timeout',
                    wall_seconds=round(time.perf_counter() - started, 2))
    record.update(returncode=done.returncode,
                  wall_seconds=round(time.perf_counter() - started, 2))
    try:
        if done.returncode != 0:
            raise RuntimeError(f'whisper-cli exited with {done.returncode}')
        timings = parse_timings(done.stderr)
        text = transcript_text(json.loads(Path(f'{output_prefix}.json').read_text('utf-8')))
    except (RuntimeError, ValueError, KeyError, OSError) as error:
        tail = '\n'.join(done.stderr.splitlines()[-3:])
        return dict(record, status='failed', error=hide_paths(f'{error}\n{tail}'.strip()))
    processing = (timings['total_ms'] - timings['load_ms']) / 1000
    errors = char_errors(chunk['reference'], text)
    return dict(record, status='completed', load_seconds=round(timings['load_ms'] / 1000, 3),
                processing_seconds=round(processing, 3),
                rtf=round(processing / chunk['seconds'], 4),
                fallbacks=timings.get('fallbacks'), decoding=timings.get('decoding'),
                anomaly_suspect=suspected_anomaly(errors), **errors)


def summarize(records, expected):
    done = [r for r in records if r['status'] == 'completed']
    failures = expected - len(done)
    result = {'expected_chunks': expected, 'completed': len(done), 'failures': failures,
              'judgeable': expected > 0 and failures / expected <= MAX_FAILURE_RATIO}
    if not done:
        return dict(result, realtime_ok=False)
    rtfs = [r['rtf'] for r in done]
    result.update(
        rtf_median=round(statistics.median(rtfs), 4),
        rtf_p95=round(percentile(rtfs, 0.95), 4),
        rtf_max=round(max(rtfs), 4),
        rtf_total=round(sum(r['processing_seconds'] for r in done)
                        / sum(r['seconds'] for r in done), 4),
        load_seconds_median=round(statistics.median(r['load_seconds'] for r in done), 3),
        cer=error_rate(sum(r['errors'] for r in done), sum(r['ref_chars'] for r in done)),
        cer_hangul=error_rate(sum(r['errors_hangul'] for r in done),
                              sum(r['ref_hangul'] for r in done)),
        fallbacks=sum(r['fallbacks'] or 0 for r in done),
        anomaly_suspects=[r['chunk_id'] for r in done if r['anomaly_suspect']])
    result['realtime_ok'] = (result['judgeable'] and result['rtf_median'] <= RTF_MEDIAN_LIMIT
                             and result['rtf_p95'] <= RTF_P95_LIMIT)
    return result


def select_config(entries):
    usable = [e for e in entries if e['summary']['completed'] == e['summary']['expected_chunks'] > 0]
    if not usable:
        raise ValueError('No screen configuration completed every chunk')
    best = min(usable, key=lambda e: e['summary']['rtf_median'])
    return {'build': best['build'], 'threads': best['threads']}


def select_model(entries):
    """Lowest CER among real-time models; within 0.5 points prefer the lower median RTF."""
    passing = [e for e in entries
               if e['summary']['realtime_ok'] and e['summary'].get('cer') is not None]
    if not passing:
        return None
    best = min(e['summary']['cer'] for e in passing)
    close = [e for e in passing if e['summary']['cer'] - best <= CER_TIE + 1e-9]
    return min(close, key=lambda e: e['summary']['rtf_median'])['model']


def top_models(entries, count=2):
    """Lowest-CER judgeable models, re-run with timestamps to size the omission risk."""
    ranked = sorted((e for e in entries
                     if e['summary']['judgeable'] and e['summary'].get('cer') is not None),
                    key=lambda e: e['summary']['cer'])
    return [e['model'] for e in ranked[:count]]


def _cell(value, digits=3):
    if value is None:
        return '-'
    return f'{value:.{digits}f}' if isinstance(value, float) else str(value)


def render_markdown(summary):
    lines = ['| 단계 | 모델 | 빌드 | 스레드 | 타임스탬프 | 완료/전체 | CER | 한글 CER | RTF 중앙값 '
             '| RTF 95% | RTF 최대 | 전체 RTF | 로딩(초) | 대체 디코딩 | 이상 의심 | 실시간 |',
             '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- '
             '| --- | --- |']
    for stage in ('screen', 'models', 'timestamp_check'):
        for entry in summary.get(stage, []):
            s = entry['summary']
            lines.append(
                f"| {stage} | {entry['model']} | {entry['build']} | {entry['threads']} "
                f"| {'사용' if entry['timestamps'] else '-nt'} "
                f"| {s['completed']}/{s['expected_chunks']} | {_cell(s.get('cer'), 4)} "
                f"| {_cell(s.get('cer_hangul'), 4)} | {_cell(s.get('rtf_median'))} "
                f"| {_cell(s.get('rtf_p95'))} | {_cell(s.get('rtf_max'))} "
                f"| {_cell(s.get('rtf_total'))} | {_cell(s.get('load_seconds_median'), 2)} "
                f"| {_cell(s.get('fallbacks'))} | {len(s.get('anomaly_suspects', []))} "
                f"| {'예' if s['realtime_ok'] else '아니오'} |")
    lines += ['', f"선택 실행 설정: {summary.get('selected_config')}",
              f"1차 후보 모델(추정): {summary.get('selected_model')}"]
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', choices=['screen', 'models', 'all'], default='all')
    parser.add_argument('--build', choices=['cpu', 'blas'], help='required with --stage models')
    parser.add_argument('--threads', type=int, help='required with --stage models')
    parser.add_argument('--models', nargs='*',
                        help='model ids; a subset makes this a diagnostic run kept in artifacts')
    parser.add_argument('--chunks', type=int,
                        help='limit chunks; makes this a diagnostic run kept in artifacts')
    parser.add_argument('--allow-battery', action='store_true',
                        help='record results on battery power; they are not comparable')
    args = parser.parse_args()
    if args.stage == 'models' and (args.build is None or args.threads is None):
        parser.error('--stage models requires --build and --threads from a screen summary')
    if args.chunks is not None and args.chunks < 1:
        parser.error('--chunks must be positive')

    runtime = load_config('stt-runtime.json')
    models = load_config('stt-models.json')
    by_id = {model['id']: model for model in models['models']}
    if unknown := sorted(set(args.models or []) - set(by_id)):
        parser.error(f"Unknown model ids: {', '.join(unknown)}")
    model_ids = args.models or [model['id'] for model in models['models']]
    fixture = json.loads((ROOT / 'evaluation/fixtures' / f'{FIXTURE_ID}.json').read_text('utf-8'))
    chunks = fixture['chunks'][:args.chunks] if args.chunks else fixture['chunks']
    runtime_root = ROOT / 'runtimes' / f"whisper-{runtime['tag']}"
    screening = args.stage in ('screen', 'all')
    comparing = args.stage in ('models', 'all')
    builds = {'cpu', 'blas'} if screening else {args.build}
    needed = ([models['screen_model']] if screening else []) + (model_ids if comparing else [])

    problems = [f'missing {build} build' for build in sorted(builds)
                if not (runtime_root / build / 'Release/whisper-cli.exe').exists()]
    for model_id in dict.fromkeys(needed):
        model = by_id[model_id]
        if not verify(ROOT / 'models/whisper' / model['filename'], model['size_bytes'],
                      model['sha256']):
            problems.append(f'model {model_id} missing or corrupt')
    problems += verify_chunks(fixture, chunk_dir())
    if problems:
        raise SystemExit('Preflight failed: ' + '; '.join(problems[:5])
                         + ' (run scripts/prepare_stt.py and scripts/stt_data.py)')
    if running := competing_processes(BLOCKING_PROCESSES):
        raise SystemExit(f"Stop other inference processes first: {', '.join(running)}")
    power = power_status()
    if power['ac_power'] is not True and not args.allow_battery:
        raise SystemExit(f'Connect AC power before benchmarking (power: {power})')
    mode = power_mode()
    if mode['ac_mode'] != 'best_performance':
        print(f"Warning: targets are judged in best_performance mode; current: {mode['ac_mode']}",
              flush=True)

    diagnostic = args.chunks is not None or bool(args.models)
    out_dir = ROOT / 'artifacts' / f'stt-bench-{time.time_ns()}'
    out_dir.mkdir(parents=True)
    summary = {'schema_version': 1, 'kind': 'stt-cpu-benchmark',
               'scope': 'whisper.cpp CPU on FLEURS Korean read-speech chunks; estimates only',
               'runtime_tag': runtime['tag'], 'fixture_id': FIXTURE_ID, 'stage': args.stage,
               'diagnostic': diagnostic, 'chunks': len(chunks),
               'audio_seconds': round(sum(c['seconds'] for c in chunks), 3),
               'criteria': {'rtf_median_max': RTF_MEDIAN_LIMIT, 'rtf_p95_max': RTF_P95_LIMIT,
                            'cer_tie': CER_TIE, 'max_failure_ratio': MAX_FAILURE_RATIO},
               'decoding': 'screen and model comparison use -nt; the two lowest-CER models are '
                           're-run with timestamps (timestamp_check)',
               'started_at': utc_now(), 'power_at_start': power, 'power_mode_at_start': mode,
               'screen': [], 'models': [], 'timestamp_check': [], 'selected_config': None,
               'selected_model': None}

    def save():
        (out_dir / 'summary.json').write_text(
            json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')

    def run_set(entries, build, threads, model_id, chunk_list, timestamps=False):
        entry = {'model': model_id, 'build': build, 'threads': threads,
                 'timestamps': timestamps, 'decoding': None, 'records': []}
        entries.append(entry)
        label = f"{model_id} {build} t{threads} {'ts' if timestamps else 'nt'}"
        folder = out_dir / label.replace(' ', '-')
        folder.mkdir()
        executable = runtime_root / build / 'Release/whisper-cli.exe'
        model_path = ROOT / 'models/whisper' / by_id[model_id]['filename']
        print(f'[{utc_now()}] {label}: {len(chunk_list)} chunks', flush=True)
        for chunk in chunk_list:
            prefix = folder / chunk['chunk_id']
            wav = chunk_dir() / f"{chunk['chunk_id']}.wav"
            command = cli_command(executable, model_path, wav, threads, prefix, timestamps)
            record = run_chunk(command, prefix, chunk)
            decoding = record.pop('decoding', None)
            entry['decoding'] = entry['decoding'] or decoding
            entry['records'].append(record)
            entry['summary'] = summarize(entry['records'], len(chunk_list))
            save()
        s = entry['summary']
        print(f"[{utc_now()}] {label}: {s['completed']}/{s['expected_chunks']} done, "
              f"rtf median {s.get('rtf_median')}, cer {s.get('cer')}", flush=True)

    status = 'completed'
    with keep_awake():
        if screening:
            for config in screen_configs():
                run_set(summary['screen'], config['build'], config['threads'],
                        models['screen_model'], chunks[:SCREEN_CHUNKS])
            try:
                summary['selected_config'] = select_config(summary['screen'])
            except ValueError as error:
                summary['error'] = str(error)
                status = 'failed'
        else:
            summary['selected_config'] = {'build': args.build, 'threads': args.threads}
        if comparing and status == 'completed':
            config = summary['selected_config']
            for model_id in model_ids:
                run_set(summary['models'], config['build'], config['threads'], model_id, chunks)
            summary['selected_model'] = select_model(summary['models'])
            for model_id in top_models(summary['models']):
                run_set(summary['timestamp_check'], config['build'], config['threads'], model_id,
                        chunks, timestamps=True)
    summary.update(status=status, finished_at=utc_now(), power_at_end=power_status(),
                   power_mode_at_end=power_mode())
    save()
    (out_dir / 'report.md').write_text(render_markdown(summary) + '\n', 'utf-8')
    print(f"Selected config: {summary['selected_config']}; model: {summary['selected_model']}",
          flush=True)
    if diagnostic:
        print('Diagnostic run: results kept in artifacts only', flush=True)
    else:
        stamp = datetime.now().strftime('%Y-%m-%d')
        result_path = unique_path(ROOT / 'evaluation/results' / f'{stamp}-stt-bench-{args.stage}.json')
        result_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')
        print(f'Summary: {result_path}', flush=True)
    print(f'Logs: {out_dir}', flush=True)
    if status != 'completed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_run_stt_bench.py -v`
Expected: PASS — `Ran 12 tests`, `OK`

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 80 tests`, `OK`

Run: `python scripts/run_stt_bench.py --stage models`
Expected: 종료 코드 2, `error: --stage models requires --build and --threads from a screen summary`

- [ ] **Step 5: 실제 바이너리로 진단 실행**

노트북 사용 중에도 실행할 수 있는 짧은 진단이다. 배터리 사용 중이면 `--allow-battery`를 붙인다. 결과는 `artifacts/`에만 남는다.

Run: `python scripts/run_stt_bench.py --stage models --build cpu --threads 4 --models small-q5_1 --chunks 2 --allow-battery`
Expected: 다음 형식의 줄이 나오고 종료 코드 0.

```
[...] small-q5_1 cpu t4 nt: 2 chunks
[...] small-q5_1 cpu t4 nt: 2/2 done, rtf median ..., cer ...
[...] small-q5_1 cpu t4 ts: 2 chunks
[...] small-q5_1 cpu t4 ts: 2/2 done, rtf median ..., cer ...
Selected config: {'build': 'cpu', 'threads': 4}; model: ...
Diagnostic run: results kept in artifacts only
Logs: ...\artifacts\stt-bench-...
```

`artifacts/stt-bench-*/summary.json`에서 `models[0].decoding`이 `{'threads': 4, ..., 'beams': 5, 'best_of': 5, 'language': 'ko'}`이고, `timestamp_check[0].timestamps`가 `true`인지 확인한다.

- [ ] **Step 6: 사용법 문서화**

`evaluation/README.md`의 `## 측정 범위` 바로 앞에 다음 절을 추가한다.

````markdown
## 로컬 STT 측정

```powershell
python scripts/prepare_stt.py
python scripts/stt_data.py
python scripts/run_stt_bench.py --stage all
```

- `prepare_stt.py`는 whisper.cpp `b5130`의 Windows x64 CPU·OpenBLAS 빌드, Whisper 모델 6종(약 7.1GB), FLEURS 한국어 test를 받아 크기·SHA-256을 검증한다. 큰 파일은 재개 가능한 구간 다운로드를 쓴다.
- `stt_data.py`는 FLEURS 문장을 이어 붙인 30초 이하 조각 60개(약 23분)를 `artifacts/stt-chunks/`에 만들고, 조각 목록·정답·출처를 `evaluation/fixtures/stt-fleurs-ko-v1.json`에 쓴다. 낭독 음성이며 강의 녹음이 아니다.
- `run_stt_bench.py`는 조각마다 `whisper-cli`를 네트워크 없이 실행한다. 실행 설정 선정(CPU·OpenBLAS × 4·8스레드, 조각 10개), 모델 비교(6종, `-nt`), CER 상위 2개 모델의 타임스탬프 비교 순서로 진행한다. RTF는 모델 로딩을 뺀 처리 시간 ÷ 조각 길이다.
- AC 전원이 아니거나 다른 llama·whisper 프로세스가 실행 중이면 시작하지 않는다. 시간 목표 판정은 전원 모드 "최고 성능"에서 한다.
- `--models`나 `--chunks`를 쓴 실행은 진단용이며 결과를 `artifacts/`에만 남긴다. 전체 실행 결과는 `evaluation/results/<날짜>-stt-bench-<단계>.json`에 저장하고, 모델 출력 전문은 커밋하지 않는다.
- 이 측정은 CPU 실시간 가능성과 모델 간 상대 비교를 추정할 뿐이며, 강의 음성·전공 용어·STT와 LLM 동시 실행 품질을 검증하지 않는다.

````

- [ ] **Step 7: 커밋**

```bash
git add scripts/run_stt_bench.py tests/test_run_stt_bench.py evaluation/README.md
git commit -m "feat: add whisper.cpp CPU benchmark runner for FLEURS Korean" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: 실측과 1차 판정 기록

**Files:**
- Create: `evaluation/results/<날짜>-stt-bench-all.json` (실행기가 생성)
- Create: `docs/validation/<날짜>-stt-bench.md`
- Create: `docs/decisions/0008-stt-candidate.md`
- Modify: `docs/ROADMAP.md`, `docs/decisions/0006-lecture-first-product-scope.md`, `README.md`

**Interfaces:**
- Consumes: Task 1~5의 도구와 자산.
- Produces: 실시간 가능 모델 목록, 1차 후보 모델(또는 없음), 타임스탬프 방식 누락 위험 수치, 로드맵 다음 작업.

- [ ] **Step 1: 사용자 확인과 사전 점검**

측정은 약 1.5~2.5시간(추정)이며 그동안 노트북을 쓰지 않아야 한다. 시작 전에 사용자에게 다음을 확인받는다: AC 전원 연결, 전원 모드 "최고 성능", 무거운 앱 종료, OneDrive 종료 허락(종료했다면 측정 후 다시 실행). 전원 모드·드라이버·앱 설정은 사용자가 바꾸며, 측정 도구는 바꾸지 않는다.

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 81 tests`, `OK`

Run: `python -c "import sys; sys.path.insert(0,'scripts'); import bench_env as E; print(E.power_status(), E.power_mode()['ac_mode'], E.competing_processes({'llama-server.exe','llama-bench.exe','llama-cli.exe','whisper-cli.exe','whisper-server.exe'}))"`
Expected: `{'ac_power': True, 'battery_percent': <값>, 'battery_saver': False} best_performance []`

사용자가 OneDrive 종료를 허락한 경우에만 실행 경로를 저장하고 종료를 요청한다.

```powershell
$onedrive = Get-Process OneDrive -ErrorAction SilentlyContinue | Select-Object -First 1
if ($onedrive) { $onedrive.Path | Out-File -Encoding utf8 artifacts\onedrive-path.txt; & $onedrive.Path /shutdown }
```

30초 뒤에도 `Get-Process OneDrive`가 남아 있으면 사용자에게 알리고 허락을 받은 뒤 `Stop-Process -Name OneDrive`로 종료한다.

실행 중 발견(2026-09-18): 첫 실측이 대기 모드(Modern Standby)로 4회 중단되어 18시간 넘게 멈췄고, 깨어난 뒤 조각 2개가 시간 초과로 기록됐다. `bench_env.keep_awake`가 절전 방지와 함께 화면 유지(`ES_DISPLAY_REQUIRED`)도 요청하도록 고쳤다(테스트 `test_keep_awake_blocks_sleep_and_display_off`). 그래도 덮개를 닫거나 사용자가 절전으로 전환하면 멈추므로, 측정 전에 다음을 사용자에게 확인한다.

- 설정 → 시스템 → 전원 및 배터리 → 화면·절전 모드에서 전원 연결 시 절전 전환을 "안 함"으로 둔다(전원 설정은 사용자가 바꾼다).
- 덮개를 열어 두고 측정 중 절전으로 전환하지 않는다.
- 측정 후 `artifacts/stt-counters-*.csv`의 시각 간격을 확인해 60초를 넘는 공백이 있으면 중단된 구간으로 보고 보고서에 적는다. 조각 기록의 `wall_seconds`가 처리 시간보다 크게 길면 같은 원인이다.

시작 직전 CPU를 많이 쓰는 프로세스를 기록한다.

```powershell
(Get-Counter '\Process(*)\% Processor Time' -SampleInterval 2).CounterSamples | Where-Object { $_.InstanceName -notin '_total', 'idle' } | Sort-Object CookedValue -Descending | Select-Object -First 8 InstanceName, @{n = 'cores'; e = { [math]::Round($_.CookedValue / 100, 2) } }
```

Expected: 프로세스별 사용 코어 수 표. 0.5코어 이상인 프로세스는 보고서의 배경 부하에 적는다.

- [ ] **Step 2: 전체 측정 실행**

측정을 백그라운드로 실행하고 콘솔 출력을 파일로 남긴다.

```bash
python scripts/run_stt_bench.py --stage all > artifacts/stt-bench-console.log 2>&1; echo "exit $?" >> artifacts/stt-bench-console.log
```

측정 프로세스가 뜬 뒤(수 초 후) 별도 백그라운드 PowerShell로 약 5초마다 CPU·온도·전력을 기록한다. 측정 프로세스가 끝나면 스스로 멈춘다.

```powershell
$csv = "artifacts\stt-counters-$(Get-Date -Format yyyyMMdd-HHmmss).csv"
'timestamp,counter,value' | Out-File -Encoding utf8 $csv
$counters = '\Processor(_Total)\% Processor Time', '\Processor Information(_Total)\% Processor Performance', '\Thermal Zone Information(*)\Temperature', '\Thermal Zone Information(*)\% Passive Limit', '\Energy Meter(*)\Power', '\Process(whisper-cli*)\% Processor Time', '\Process(explorer*)\% Processor Time', '\Process(searchindexer*)\% Processor Time', '\Process(onedrive*)\% Processor Time'
while (Get-CimInstance Win32_Process -Filter "Name='python.exe'" | Where-Object CommandLine -like '*run_stt_bench.py*') {
  $sample = Get-Counter -Counter $counters -ErrorAction SilentlyContinue
  foreach ($x in $sample.CounterSamples) { '{0},{1},{2}' -f $sample.Timestamp.ToString('o'), ($x.Path -replace '^\\\\[^\\]+', ''), $x.CookedValue | Out-File -Append -Encoding utf8 $csv }
  Start-Sleep -Seconds 4
}
```

온도 값은 켈빈이다(섭씨 = 값 − 273). `% Passive Limit`이 100 미만이면 열 제한이 걸린 것이다. 전력 값은 밀리와트다.

Expected: 콘솔 로그에 실행 묶음마다 시작·완료 줄이 있고, 끝부분이 다음 형식이다(값은 예시).

```
Selected config: {'build': 'blas', 'threads': 8}; model: large-v3-turbo-q5_0
Summary: <repo>\evaluation\results\<날짜>-stt-bench-all.json
Logs: <repo>\artifacts\stt-bench-<숫자>
exit 0
```

판단 규칙:
- `exit 0`이면 Step 3으로 간다.
- `exit 1`(실행 설정 선정 실패)이면 요약의 `error`와 `screen[].records[].error`를 확인한다. 원인이 일시적이면 한 번만 다시 실행하고, 다시 실패하면 드라이버·설정을 건드리지 말고 Step 4~5에 실패로 기록한 뒤 사용자에게 보고한다.
- `Preflight failed`, `Stop other inference processes`, `Connect AC power`는 측정 전 중단이다. 원인을 해결하고 다시 실행한다.
- 측정이 끝나면, OneDrive를 종료했던 경우 `Start-Process (Get-Content artifacts\onedrive-path.txt) -ArgumentList '/background'`로 다시 실행한다.

- [ ] **Step 3: 결과 확인**

Run: `python -c "import json,sys; s=json.load(open(sys.argv[1],encoding='utf-8')); [print(e['model'], e['build'], e['threads'], e['timestamps'], {k: e['summary'].get(k) for k in ('completed','expected_chunks','cer','cer_hangul','rtf_median','rtf_p95','rtf_total','realtime_ok')}) for g in ('screen','models','timestamp_check') for e in s[g]]; print(s['selected_config'], s['selected_model'], s['power_mode_at_start']['ac_mode'])" evaluation/results/<날짜>-stt-bench-all.json`
Expected: 실행 묶음별 한 줄과 선정 결과. 표는 `artifacts/stt-bench-*/report.md`에 있다.

이상 의심 조각이 많은 모델은 `artifacts/stt-bench-*/<모델>-*/<조각>.json` 출력과 조각 목록의 정답을 비교해 누락·반복 유형을 확인한다(출력 전문은 커밋하지 않는다).

- [ ] **Step 4: 보고서 작성**

`docs/validation/<날짜>-stt-bench.md`를 다음 구조로 쓴다. 수치는 요약 JSON과 `report.md`에서 옮기고 추정값에는 "추정"을 붙인다.

````markdown
# 로컬 STT 측정: whisper.cpp CPU·FLEURS 한국어

- 날짜·범위·결과 파일 링크(요약 JSON, 조각 목록, 설계 문서, 실행 계획)
- 범위: FLEURS 한국어 낭독 음성 조각 60개(1,406초). 강의·전공 용어·STT와 LLM 동시 실행·앱 지연시간 시험이 아님

## 요약
실시간 가능 모델, 1차 후보(또는 없음), 가장 낮은 CER, 타임스탬프 방식 누락 위험을 3~5문장으로 적는다.

## 측정 조건
전원 상태·전원 모드(시작·끝), 배경 부하, 온도, 빌드 태그, 디코딩 설정(빔·후보·온도 대체), 조각 길이 분포, 소요 시간을 표로 적는다.

## 실행 설정 선정
`report.md`의 screen 행과 선택 이유(모든 조각 완료 조합 중 RTF 중앙값 최저)를 적는다.

## 모델 비교
`report.md`의 models 행을 옮기고, 모델 크기·양자화에 따른 CER·RTF 차이와 `cer`·`cer_hangul` 차이를 해석한다.

## 타임스탬프 방식 비교
timestamp_check 행과 같은 모델의 -nt 결과를 나란히 두고 CER·이상 의심 조각 수 차이를 적는다. 앱에서 타임스탬프를 얻는 방식(예: 음성 구간 검출 후 구간별 전사)에 대한 시사점을 적는다.

## 판정과 다음 결정
1차 후보와 그 근거, 속도 기준을 통과한 모델이 없을 때의 선택지(GPU·NPU 경로, 모델 조정), 강의 녹음 평가 필요성을 적는다.

## 한계
낭독 음성, 문장 이어 붙이기, 짧은 조각의 RTF 과대 추정, 배경 부하, 1회 실행, 공개 데이터 정답의 원어·숫자 표기, 강의 녹음 미평가를 적는다.
````

- [ ] **Step 5: 결정 기록 초안과 문서 갱신**

`docs/decisions/0008-stt-candidate.md`를 만든다. 상태는 "초안: 공개 낭독 음성 기준 1차 후보, 강의 녹음 평가 전"으로 둔다. 내용은 선정 모델(또는 없음), 실행 설정, 판정 근거와 기준, 타임스탬프 방식 누락 위험, 라이선스(whisper.cpp MIT, 모델 MIT, OpenBLAS 재배포 전 확인 필요), 재검토 조건(강의 녹음 평가, STT·LLM 동시 실행, GPU·NPU 경로)이다.

`docs/ROADMAP.md`:
- M0 남은 작업 2 끝에 `결과(추정): <1차 후보 또는 없음>, [보고서](validation/<날짜>-stt-bench.md), [결정 초안](decisions/0008-stt-candidate.md).`를 붙인다. 강의 녹음·전공 용어 평가가 남아 있으므로 체크박스는 `[ ]`로 둔다.
- `## 7. 바로 다음 작업`: 1차 후보가 있으면 "녹음 중 STT·LLM 동시 실행 측정(남은 작업 3)"을, 없으면 "사용자 결정: GPU·NPU 경로 또는 모델 조정"을 다음 작업으로 둔다.

`docs/decisions/0006-lecture-first-product-scope.md` 5절의 "RAM 16GB 노트북에서 STT와 LLM을 동시에…" 항목 끝에 `STT 단독 CPU 측정 결과는 [보고서](../validation/<날짜>-stt-bench.md)에 기록했다.`를 붙인다.

`README.md` 문서 목록의 `llama-bench 재측정` 줄 다음에 `- [로컬 STT 측정](docs/validation/<날짜>-stt-bench.md)`을 추가한다.

- [ ] **Step 6: 검증 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 81 tests`, `OK`

다음 검사 스크립트를 `artifacts/check_docs.py`로 저장하고 저장소 최상위에서 실행한다(`python artifacts/check_docs.py`).

```python
import json
import pathlib
import re

root = pathlib.Path('.').resolve()
needles = {str(root), root.as_posix(), str(pathlib.Path.home()), pathlib.Path.home().as_posix()}
needles |= {json.dumps(n)[1:-1] for n in needles}
committed = sorted(root.glob('evaluation/results/*stt-bench*.json')) + [
    root / 'evaluation/fixtures/stt-fleurs-ko-v1.json']
for path in committed:
    json.loads(path.read_text('utf-8'))
leaks = [path.name for path in committed if any(n in path.read_text('utf-8') for n in needles)]
print('local paths:', leaks or 'none')
bad = []
for md in list(root.glob('*.md')) + list(root.glob('docs/**/*.md')) + list(root.glob('evaluation/*.md')):
    for target in re.findall(r'\]\(([^)\s]+)\)', md.read_text('utf-8')):
        if not target.startswith(('http', '#')) and '<' not in target \
                and not (md.parent / target.split('#')[0]).resolve().exists():
            bad.append((md.name, target))
print('broken links:', bad or 'none')
```

Expected: `local paths: none`, `broken links: none`

Run: `git status --short`
Expected: 이번 Task의 결과·문서 파일만 변경되어 있고 `artifacts/`·`models/`·`downloads/`·`runtimes/`는 표시되지 않는다.

```bash
git add evaluation/results/<날짜>-stt-bench-all.json docs/validation/<날짜>-stt-bench.md docs/decisions/0008-stt-candidate.md docs/ROADMAP.md docs/decisions/0006-lecture-first-product-scope.md README.md
git commit -m "test: measure whisper.cpp CPU speech recognition on FLEURS Korean" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 7: 사용자 보고**

작업·결과·다음 형식으로 보고한다. 결과에는 실시간 가능 모델, 1차 후보와 CER·RTF, 타임스탬프 방식 누락 위험, 측정 조건을 포함한다. 후보가 없으면 선택지를 제시하고 결정을 기다린다.
