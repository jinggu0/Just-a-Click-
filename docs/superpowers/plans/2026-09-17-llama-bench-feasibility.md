# llama-bench 처리량 측정과 시간 목표 실현 가능성 1차 판정 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 고정된 llama.cpp b10994의 `llama-bench`로 스키마 제약 없는 입력 처리·생성 속도를 백엔드·스레드·Flash Attention·컨텍스트 깊이별로 측정한다. 이어서 서버의 출력 제약(없음·JSON·스키마)별 생성 속도와 비교해 속도 저하 원인을 분리하고, [결정 0006](../../decisions/0006-lecture-first-product-scope.md)의 녹음 종료 후 5분 노트·다시 요약 1시간 목표를 1차 판정한다.

**Architecture:** 표준 라이브러리 Python 모듈 세 개를 추가한다. 결과 파서(`llama_bench_results.py`), 처리량 곡선·작업량 가정으로 시간을 추정하는 추정기(`feasibility.py`), 측정 조합을 별도 프로세스로 실행하고 요약 JSON을 남기는 실행기(`run_llama_bench.py`)다. 기존 `benchmark_llm.py`에는 출력 제약만 바꾸는 진단 옵션을 추가하고, 측정 후 결과·보고서·로드맵을 갱신한다.

**Tech Stack:** Python 3.11 표준 라이브러리(`unittest`, `subprocess`, `ctypes`, `statistics`), llama.cpp b10994(`llama-bench.exe`, `llama-server.exe`, CPU·Vulkan 빌드), Qwen3-8B Q5_K_M.

## Global Constraints

- Python 3.11 이상 표준 라이브러리만 사용한다. pytest 등 새 의존성을 추가하지 않는다. 전체 테스트 명령은 `python -m unittest discover -s tests -v`다.
- 모델은 `config/llm-model.json`의 `Qwen3-8B-Q5_K_M.gguf`, 런타임은 `config/llama-runtime.json`의 `b10994`를 그대로 쓴다. 다른 모델·양자화로 바꾸지 않는다.
- Windows 드라이버·TDR·레지스트리·전원 설정을 바꾸지 않는다. 절전 방지는 측정 프로세스 실행 중에만 `SetThreadExecutionState`로 요청한다. 입력을 잘라 성공으로 처리하지 않는다.
- 측정은 AC 전원 연결, 다른 `llama-*` 프로세스가 없는 상태에서 조건별 3회 반복하고 중앙값과 범위를 기록한다.
- `artifacts/`의 원출력·로그는 커밋하지 않는다. `evaluation/results/`에는 합성 토큰 측정 요약만 커밋하며 로컬 경로·인증 토큰을 넣지 않는다.
- `llama-bench` 수치는 원시 처리량이다. 앱 지연시간·요약 품질 통과로 표현하지 않으며, 시간 계산은 항상 "추정"으로 표시한다.
- 시간 목표는 녹음 종료 후 5분 노트 300초, 라이브러리 다시 요약 3,600초다(결정 0006). 결과가 미달이어도 모델·목표를 임의로 바꾸지 않고 사용자 결정으로 넘긴다.
- 로컬 서버는 기존 `benchmark_llm.py`처럼 `127.0.0.1` 전용·임의 토큰을 유지한다.
- 문서는 한국어, 코드와 주석은 기존 스크립트처럼 영어로 작성한다.
- 각 Task는 검증 후 해당 경로만 stage하여 커밋하고, 메시지 끝에 `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`를 붙인다. push하지 않는다.

---

## 사전 확인 사실 (2026-09-17, 계획 작성 중 직접 확인)

- `runtimes/b10994/{cpu,vulkan}/llama-bench.exe` 도움말에서 `-d/--n-depth`, `-fa on|off|auto`, `-ub`, `-r`, `-o jsonl`, `--progress`, `--no-warmup`을 확인했다.
- Vulkan 빌드는 `Intel(R) Arc(TM) 140V GPU (16GB)`를 인식한다.
- `-o jsonl`은 측정이 끝날 때마다 한 줄씩 출력하므로 프로세스가 중간에 죽어도 앞선 결과가 남는다. `-o json`은 배열이 닫히지 않아 전체를 잃을 수 있다.
- JSONL 필드: `backends`, `n_threads`, `n_batch`, `n_ubatch`, `n_gpu_layers`, `flash_attn`(`auto=-1`, `off=0`, `on=1` 추정), `n_prompt`, `n_gen`, `n_depth`, `samples_ts`, `avg_ts`, `model_filename`(절대 경로), `model_type`, `cpu_info`, `gpu_info`, `build_number`, `build_commit`, `test_time`. 이 중 `off=0`·`auto=-1`은 실제 출력으로 확인했고 `on=1`은 미확인이다.
- `-d` 깊이의 선행 입력 처리는 시간 측정에서 제외되며, 로그상 반복 시 캐시된다(`depth run 1/1 (cached)`).
- `tasklist /FO CSV /NH`는 `encoding='oem'`으로 읽을 수 있고, `GetSystemPowerStatus`는 현재 AC 연결을 보고한다.
- 이 계획의 코드는 저장소 사본에서 먼저 구현했다. 기존 테스트 11개와 새 테스트 28개를 합친 39개가 통과했고, 실제 `llama-bench.exe`(CPU, 4토큰)로 실행기의 프로세스 처리·파싱을 확인했다.

## 측정 설계

### 측정 단계

| 단계 | 작업 이름 | 조건 | 예상 시간(추정) |
| --- | --- | --- | --- |
| screen | `cpu-t4`, `cpu-t8` | CPU 빌드, `-ngl 0`, FA auto, pp512·tg128, 깊이 0 | 각 약 6분 |
| screen | `vulkan-t{4,8}-fa-{off,on}` | Vulkan 빌드, `-ngl 99`, ubatch 512, pp512·tg128, 깊이 0·2,048 | 각 약 5분 |
| depth | `vulkan-depth-ub128` | screen에서 고른 스레드·FA, ubatch 128, 깊이 0·2,048·8,192 | 약 12분 |
| depth | `vulkan-depth-ub512` | 같은 설정, ubatch 512, 깊이 8,192. **실패 허용 위험 시험** | 약 7분 또는 조기 실패 |
| server | `benchmark_llm.py` × 3 | 같은 엄격한 프롬프트, 출력 제약 `none`·`json`·`schema`, 각 3회 | 약 15분 |

- screen 선택 규칙: Vulkan 깊이 0의 tg128 중앙값이 가장 높은 스레드·FA 조합을 고른다. 동률이면 pp512 중앙값이 높은 쪽을 고른다. 생성 속도가 이미 확인된 주 병목이기 때문이다.
- ubatch 512의 8,192 깊이는 서버 시험에서 `vk::Device::getFenceStatus: ErrorDeviceLost`가 났던 조건이다. 가장 마지막에 별도 프로세스로 실행하고, 실패하면 요약에 기록만 한다.

### 시간 추정 모델

- 처리량 곡선: 깊이별 pp512·tg128 중앙값을 깊이에 따라 선형 보간한다. 측정 범위 밖은 끝값으로 고정하고, N토큰의 처리 시간은 64토큰 단위로 적분한다.
- 입력 처리 곡선: 목표 계산에 필요한 깊이(최대 3,000토큰)까지 측정된 ubatch 곡선을 우선 쓰고, 그중 더 빠른 것을 고른다.
- 서버 보정 계수: `schema` 서버 실행의 생성 속도 ÷ 같은 깊이의 `llama-bench` 생성 속도(중앙값)다. 샘플링·문법 제약·서버 부담을 한 값에 반영하며, 이 값을 생성 곡선에 곱한다.

가정값(결정 0006 기반, 실제 강의로 측정하지 않음):

| 항목 | 값 |
| --- | --- |
| 발화 속도 | 초당 2.5·4.0음절 |
| 음절당 토큰 | 0.9 (합성 문장 1건: 249음절 → 225토큰) |
| 구간 길이 | 300초 |
| 구간 입력 고정 부담(지시·스키마) | 900토큰 |
| 구간 출력 | 300토큰 |
| 최종 통합 입력·출력 | 3,000토큰 · 400토큰 |
| 녹음 종료 시 밀린 구간 | 2개(진행 중 1 + 대기 1, 보수적) |
| 녹음 중 LLM 점유율 한도 | 구간 길이의 0.5 (나머지는 STT 몫) |
| 강의 길이 | 2·3시간(대표), 4시간(최대, 참고) |

계산식:
- 구간 처리 시간 = 구간 입력 처리 + 구간 출력 생성
- 종료 후 시간 = 2 × 구간 처리 시간 + 최종 통합 시간
- 다시 요약 시간 = ⌈강의 초 ÷ 300⌉ × 구간 처리 시간 + 최종 통합 시간

판정 기준(대표 길이 2·3시간 × 두 발화 속도, 모두 추정):
- 모든 시나리오가 점유율 ≤ 0.5, 종료 후 ≤ 300초, 다시 요약 ≤ 3,600초를 만족하면 `feasible_estimate`.
- 일부만 만족하면 `borderline_estimate`, 하나도 만족하지 못하면 `infeasible_estimate`.
- 각 시나리오에 목표를 맞추는 데 필요한 균일 생성 속도(`required_tg_tps_*`)도 함께 기록한다.

### 범위 밖

SYCL 빌드 비교(로드맵 M0 남은 작업 6), STT 측정(작업 2·3), 실제 강의 전사문 처리, Q4_K_M 비교, 드라이버·TDR 조정은 이 계획에 포함하지 않는다.

## File Structure

| 경로 | 구분 | 책임 |
| --- | --- | --- |
| `scripts/llama_bench_results.py` | 생성 | JSONL 파싱, 레코드 정규화(로컬 경로 제거), 설정 선택 |
| `scripts/feasibility.py` | 생성 | 처리량 곡선, 작업량 추정, 판정, 서버 보정 계수, 한국어 표, CLI |
| `scripts/run_llama_bench.py` | 생성 | 측정 조합, 사전 점검(모델·프로세스·전원), 절전 방지, 작업 실행·실패 기록, 요약 저장 |
| `scripts/benchmark_llm.py` | 수정 | `--threads`, `--response-format` 진단 옵션, 보고서 `response_format` 필드 |
| `tests/test_llama_bench_results.py` | 생성 | 파서·정규화·선택 테스트 |
| `tests/test_feasibility.py` | 생성 | 보간·적분·추정·판정·보정 계수·표 테스트 |
| `tests/test_run_llama_bench.py` | 생성 | 작업 조합·명령·환경 점검·실패 처리 테스트 |
| `tests/test_benchmark_options.py` | 생성 | 출력 제약 선택 테스트 |
| `evaluation/README.md` | 수정 | 새 측정·추정 사용법 |
| `evaluation/results/<날짜>-*.json` | 생성(Task 5) | 측정 요약, 서버 제약 비교, 추정 결과 |
| `docs/validation/<날짜>-llama-bench.md` | 생성(Task 5) | 측정 보고서 |
| `docs/ROADMAP.md`, `docs/LLM-SPECIALIZATION.md`, `docs/decisions/0006-lecture-first-product-scope.md`, `README.md` | 수정(Task 5) | 판정 결과와 다음 작업 반영 |

`<날짜>`는 측정을 실행한 날짜(`YYYY-MM-DD`)이며, `run_llama_bench.py`가 출력하는 `Summary:` 경로의 날짜와 같다.

---

### Task 1: llama-bench 결과 파서

**Files:**
- Create: `scripts/llama_bench_results.py`
- Test: `tests/test_llama_bench_results.py`

**Interfaces:**
- Consumes: 없음.
- Produces:
  - `parse_jsonl(text: str) -> tuple[list[dict], int]`: `(원본 레코드 목록, 깨진 JSON 줄 수)`를 반환한다.
  - `normalize(raw: dict, runtime: str) -> dict`: 반환 키는 `runtime`, `backend`, `threads`, `flash_attn`(`'auto'|'off'|'on'`), `batch`, `ubatch`, `gpu_layers`, `test`(`'pp'|'tg'`), `tokens`, `depth`, `median_tps`, `min_tps`, `max_tps`, `samples_tps`, `repetitions`, `build`, `model`(파일명만), `model_type`, `cpu_info`, `gpu_info`, `test_time`다.
  - `select_best(records: list[dict], runtime: str = 'vulkan') -> dict`: `{'threads': int, 'flash_attn': str}`를 반환한다.

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_llama_bench_results.py`:

```python
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
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_llama_bench_results.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'llama_bench_results'`

- [ ] **Step 3: 최소 구현 작성**

`scripts/llama_bench_results.py`:

```python
"""Parse llama-bench JSONL. Raw llama.cpp throughput only, never product latency."""
import json
from statistics import median

FLASH_ATTN = {-1: 'auto', 0: 'off', 1: 'on'}


def parse_jsonl(text):
    """Return (raw_records, malformed_line_count); a crash can leave a partial last line."""
    records, malformed = [], 0
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith('{'):
            continue
        try:
            records.append(json.loads(line))
        except ValueError:
            malformed += 1
    return records, malformed


def normalize(raw, runtime):
    """Flatten one llama-bench record; `runtime` is the pinned build folder we executed."""
    n_prompt, n_gen = raw['n_prompt'], raw['n_gen']
    if n_prompt > 0 and n_gen == 0:
        test, tokens = 'pp', n_prompt
    elif n_gen > 0 and n_prompt == 0:
        test, tokens = 'tg', n_gen
    else:
        raise ValueError('Combined prompt+generation tests are not part of this benchmark')
    samples = raw['samples_ts']
    if not samples:
        raise ValueError('llama-bench record has no samples')
    return {
        'runtime': runtime, 'backend': raw['backends'], 'threads': raw['n_threads'],
        'flash_attn': FLASH_ATTN[raw['flash_attn']], 'batch': raw['n_batch'],
        'ubatch': raw['n_ubatch'], 'gpu_layers': raw['n_gpu_layers'],
        'test': test, 'tokens': tokens, 'depth': raw['n_depth'],
        'median_tps': median(samples), 'min_tps': min(samples), 'max_tps': max(samples),
        'samples_tps': samples, 'repetitions': len(samples),
        'build': f"{raw['build_number']}/{raw['build_commit']}",
        # Keep only the file name so committed results carry no local paths.
        'model': raw['model_filename'].replace('\\', '/').rsplit('/', 1)[-1],
        'model_type': raw['model_type'], 'cpu_info': raw['cpu_info'],
        'gpu_info': raw['gpu_info'], 'test_time': raw['test_time']}


def select_best(records, runtime='vulkan'):
    """Fastest depth-0 generation wins (decode is the measured bottleneck); prompt speed breaks ties."""
    candidates = [r for r in records
                  if r['runtime'] == runtime and r['test'] == 'tg' and r['depth'] == 0]
    if not candidates:
        raise ValueError(f'No depth-0 generation results for {runtime}')

    def prompt_rate(tg):
        return max((r['median_tps'] for r in records
                    if r['runtime'] == runtime and r['test'] == 'pp' and r['tokens'] == 512
                    and r['depth'] == 0 and r['threads'] == tg['threads']
                    and r['flash_attn'] == tg['flash_attn']), default=0.0)

    best = max(candidates, key=lambda r: (r['median_tps'], prompt_rate(r)))
    return {'threads': best['threads'], 'flash_attn': best['flash_attn']}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_llama_bench_results.py -v`
Expected: PASS — `Ran 5 tests`, `OK`

- [ ] **Step 5: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 16 tests`, `OK`

```bash
git add scripts/llama_bench_results.py tests/test_llama_bench_results.py
git commit -m "test: parse llama-bench JSONL throughput records" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: 시간 목표 추정기

**Files:**
- Create: `scripts/feasibility.py`
- Test: `tests/test_feasibility.py`

**Interfaces:**
- Consumes: Task 1의 `normalize()` 레코드 형식과 `llama_bench_results.normalize` 함수(테스트 데이터 생성용).
- Consumes(데이터): Task 4의 `benchmark_llm.py` 보고서 JSON. `report['response_format'] == 'schema'`이고 `report['runs'][i]['timings']`에 `prompt_n`, `predicted_n`, `predicted_per_second`가 있다.
- Produces:
  - `TARGETS: dict`, `ASSUMPTIONS: dict`
  - `rate_at(points, depth) -> float`, `seconds(points, start, tokens, step=64) -> float`
  - `curve(records, test, runtime, threads, flash_attn, ubatch=None) -> list[list]`: `[[depth, tps], ...]`
  - `prompt_curve(records, runtime, threads, flash_attn, need_depth) -> dict`: `{'ubatch', 'points', 'extrapolated'}`
  - `estimate(pp, tg, a=ASSUMPTIONS, tg_factor=1.0) -> list[dict]`, `verdict(scenarios, representative_hours) -> str`
  - `estimate_from_records(records, selected, tg_factor=1.0, a=ASSUMPTIONS) -> dict`: 키는 `scope`, `selected`, `targets`, `assumptions`, `tg_factor`, `prompt_curve`, `generation_curve`, `scenarios`, `verdict`
  - `server_factor(reports, tg) -> float`, `render_markdown(records, result) -> str`
  - CLI: `python scripts/feasibility.py <summary.json> [--server-report <report.json>...] [--output <json>] [--markdown <md>]`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_feasibility.py`:

```python
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import feasibility as F  # noqa: E402
import llama_bench_results as L  # noqa: E402


def bench(runtime='vulkan', test='tg', depth=0, rate=5.0, ubatch=512, threads=8, fa=1):
    tokens = {'n_prompt': 512, 'n_gen': 0} if test == 'pp' else {'n_prompt': 0, 'n_gen': 128}
    return L.normalize({
        'build_commit': '0a8b29a60', 'build_number': 10994, 'backends': 'Vulkan',
        'cpu_info': 'cpu', 'gpu_info': 'gpu',
        'model_filename': 'Qwen3-8B-Q5_K_M.gguf', 'model_type': 'qwen3 8B Q5_K - Medium',
        'n_batch': 2048, 'n_ubatch': ubatch, 'n_threads': threads, 'n_gpu_layers': 99,
        'flash_attn': fa, 'n_depth': depth, 'test_time': '2026-09-17T00:00:00Z',
        'samples_ts': [rate], **tokens}, runtime)


class CurveMathTests(unittest.TestCase):
    def test_rate_interpolates_and_clamps(self):
        points = [[0, 100.0], [2048, 50.0], [8192, 20.0]]
        self.assertEqual(F.rate_at(points, -5), 100.0)
        self.assertEqual(F.rate_at(points, 1024), 75.0)
        self.assertEqual(F.rate_at(points, 5120), 35.0)
        self.assertEqual(F.rate_at(points, 10000), 20.0)

    def test_seconds_integrates_declining_rate(self):
        self.assertAlmostEqual(F.seconds([[0, 10.0]], 0, 100), 10.0)
        # Exact integral of 1 / (100 - 0.05 d) over 0..1000 is 20 ln 2.
        self.assertAlmostEqual(F.seconds([[0, 100.0], [1000, 50.0]], 0, 1000), 13.863, delta=0.01)

    def test_prompt_curve_prefers_covering_then_faster_ubatch(self):
        records = [bench(test='pp', depth=0, rate=100.0, ubatch=512),
                   bench(test='pp', depth=2048, rate=60.0, ubatch=512),
                   bench(test='pp', depth=0, rate=80.0, ubatch=128),
                   bench(test='pp', depth=2048, rate=50.0, ubatch=128),
                   bench(test='pp', depth=8192, rate=20.0, ubatch=128)]
        deep = F.prompt_curve(records, 'vulkan', 8, 'on', need_depth=3000)
        self.assertEqual((deep['ubatch'], deep['extrapolated']), (128, False))
        short = F.prompt_curve(records, 'vulkan', 8, 'on', need_depth=2000)
        self.assertEqual((short['ubatch'], short['points']), (512, [[0, 100.0], [2048, 60.0]]))


class EstimateTests(unittest.TestCase):
    def test_flat_rates_match_hand_calculation(self):
        scenarios = F.estimate([[0, 100.0]], [[0, 5.0]])
        first = scenarios[0]
        self.assertEqual((first['lecture_hours'], first['syllables_per_second']), (2, 2.5))
        self.assertEqual(first['window_input_tokens'], 1575)
        self.assertEqual(first['window_seconds'], 75.8)
        self.assertEqual(first['post_recording_seconds'], 261.5)
        self.assertEqual(first['library_resummary_seconds'], 1928.0)
        self.assertEqual(first['required_tg_tps_post'], 4.19)
        self.assertEqual(first['required_tg_tps_resummary'], 2.38)
        self.assertEqual(F.verdict(scenarios, [2, 3]), 'feasible_estimate')
        four_hours = [s for s in scenarios if s['lecture_hours'] == 4]
        self.assertFalse(any(s['library_resummary_within_target'] for s in four_hours))

    def test_verdict_degrades_with_slower_generation(self):
        borderline = F.estimate([[0, 100.0]], [[0, 4.2]])
        self.assertEqual(F.verdict(borderline, [2, 3]), 'borderline_estimate')
        slow = F.estimate([[0, 100.0]], [[0, 5.0]], tg_factor=0.5)
        self.assertEqual(F.verdict(slow, [2, 3]), 'infeasible_estimate')

    def test_prompt_time_alone_can_exceed_budget(self):
        self.assertIsNone(F.required_rate(100, 300, 301))

    def test_estimate_from_records_reports_selection(self):
        records = [bench(test='tg', depth=0, rate=5.0), bench(test='tg', depth=8192, rate=4.0),
                   bench(test='pp', depth=0, rate=100.0), bench(test='pp', depth=8192, rate=40.0),
                   bench(runtime='cpu', test='tg', rate=50.0)]
        result = F.estimate_from_records(records, {'threads': 8, 'flash_attn': 'on'})
        self.assertEqual(result['generation_curve'], [[0, 5.0], [8192, 4.0]])
        self.assertEqual(result['prompt_curve']['ubatch'], 512)
        self.assertIn(result['verdict'], {'feasible_estimate', 'borderline_estimate',
                                          'infeasible_estimate'})
        table = F.render_markdown(records, result)
        self.assertIn('판정(추정)', table)
        self.assertIn('| vulkan | 8 | on | 512 | pp512 | 8192 | 40.00 | 40.00~40.00 |', table)


class ServerFactorTests(unittest.TestCase):
    def test_factor_compares_at_same_depth(self):
        report = {'response_format': 'schema', 'runs': [
            {'timings': {'prompt_n': 926, 'predicted_n': 329, 'predicted_per_second': 2.92}}]}
        factor = F.server_factor([report], [[0, 5.0], [2048, 4.0]])
        self.assertAlmostEqual(factor, 2.92 / (5.0 - 1090.5 / 2048), places=6)

    def test_unconstrained_reports_rejected(self):
        with self.assertRaises(ValueError):
            F.server_factor([{'response_format': 'none', 'runs': []}], [[0, 5.0]])
        with self.assertRaises(ValueError):
            F.server_factor([{'response_format': 'schema', 'runs': []}], [[0, 5.0]])


if __name__ == '__main__':
    unittest.main()
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_feasibility.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'feasibility'`

- [ ] **Step 3: 최소 구현 작성**

`scripts/feasibility.py`:

```python
"""First-pass latency estimate from raw throughput. An estimate, never a release verdict."""
import argparse
import json
import math
from pathlib import Path
from statistics import median

TARGETS = {'post_recording_seconds': 300, 'library_resummary_seconds': 3600}
# Workload assumptions from decision 0006; none of these are measured on real lectures yet.
ASSUMPTIONS = {
    'syllables_per_second': [2.5, 4.0],
    'tokens_per_syllable': 0.9,
    'window_seconds': 300,
    'window_overhead_tokens': 900,
    'window_output_tokens': 300,
    'final_input_tokens': 3000,
    'final_output_tokens': 400,
    'backlog_windows': 2,
    'max_live_duty_ratio': 0.5,
    'lecture_hours': [2, 3, 4],
    'representative_hours': [2, 3],
    'step_tokens': 64,
}
PROMPT_TOKENS, GEN_TOKENS = 512, 128


def rate_at(points, depth):
    """Piecewise-linear tokens/s between measured depths; flat beyond both ends."""
    if depth <= points[0][0]:
        return points[0][1]
    for (d0, r0), (d1, r1) in zip(points, points[1:]):
        if depth <= d1:
            return r0 + (r1 - r0) * (depth - d0) / (d1 - d0)
    return points[-1][1]


def seconds(points, start, tokens, step=64):
    """Time to process `tokens` tokens beginning at context depth `start`."""
    total, done = 0.0, 0
    while done < tokens:
        n = min(step, tokens - done)
        total += n / rate_at(points, start + done + n / 2)
        done += n
    return total


def curve(records, test, runtime, threads, flash_attn, ubatch=None):
    """Median tokens/s per depth for one configuration, sorted by depth."""
    size = PROMPT_TOKENS if test == 'pp' else GEN_TOKENS
    by_depth = {}
    for r in records:
        if (r['runtime'], r['test'], r['tokens'], r['threads'], r['flash_attn']) != (
                runtime, test, size, threads, flash_attn):
            continue
        if ubatch is not None and r['ubatch'] != ubatch:
            continue
        by_depth.setdefault(r['depth'], []).append(r['median_tps'])
    if not by_depth:
        raise ValueError(f'No {test} measurements for {runtime} t{threads} fa={flash_attn}')
    return [[depth, median(values)] for depth, values in sorted(by_depth.items())]


def prompt_curve(records, runtime, threads, flash_attn, need_depth):
    """Prefer ubatch curves that reach `need_depth`; among them the fastest over that span."""
    options = []
    for ubatch in sorted({r['ubatch'] for r in records if r['runtime'] == runtime}):
        try:
            points = curve(records, 'pp', runtime, threads, flash_attn, ubatch)
        except ValueError:
            continue
        covered = points[-1][0] + PROMPT_TOKENS >= need_depth
        options.append((not covered, seconds(points, 0, need_depth), ubatch, points))
    if not options:
        raise ValueError('No prompt-processing curve for the selected configuration')
    missing, _, ubatch, points = min(options, key=lambda option: option[:2])
    return {'ubatch': ubatch, 'points': points, 'extrapolated': missing}


def required_rate(output_tokens, budget_seconds, prompt_seconds):
    """Flat decode speed that would fit the budget; None when prompt time alone exceeds it."""
    remaining = budget_seconds - prompt_seconds
    return round(output_tokens / remaining, 2) if remaining > 0 else None


def window_input_tokens(syllables_per_second, a):
    return round(a['window_overhead_tokens']
                 + syllables_per_second * a['window_seconds'] * a['tokens_per_syllable'])


def needed_depth(a):
    """Deepest prompt the time targets rely on."""
    return max([a['final_input_tokens']]
               + [window_input_tokens(sps, a) for sps in a['syllables_per_second']])


def estimate(pp, tg, a=ASSUMPTIONS, tg_factor=1.0):
    tg = [[depth, rate * tg_factor] for depth, rate in tg]
    step = a['step_tokens']
    final_in, final_out = a['final_input_tokens'], a['final_output_tokens']
    final_prompt = seconds(pp, 0, final_in, step)
    final = final_prompt + seconds(tg, final_in, final_out, step)
    scenarios = []
    for sps in a['syllables_per_second']:
        n_in = window_input_tokens(sps, a)
        n_out = a['window_output_tokens']
        window_prompt = seconds(pp, 0, n_in, step)
        window = window_prompt + seconds(tg, n_in, n_out, step)
        backlog = a['backlog_windows']
        post = backlog * window + final
        duty = window / a['window_seconds']
        for hours in a['lecture_hours']:
            windows = math.ceil(hours * 3600 / a['window_seconds'])
            resummary = windows * window + final
            scenarios.append({
                'lecture_hours': hours, 'syllables_per_second': sps,
                'window_input_tokens': n_in, 'window_seconds': round(window, 1),
                'live_duty_ratio': round(duty, 3),
                'post_recording_seconds': round(post, 1),
                'library_resummary_seconds': round(resummary, 1),
                'live_keeps_up': duty <= a['max_live_duty_ratio'],
                'post_recording_within_target': post <= TARGETS['post_recording_seconds'],
                'library_resummary_within_target':
                    resummary <= TARGETS['library_resummary_seconds'],
                'required_tg_tps_post': required_rate(
                    backlog * n_out + final_out, TARGETS['post_recording_seconds'],
                    backlog * window_prompt + final_prompt),
                'required_tg_tps_resummary': required_rate(
                    windows * n_out + final_out, TARGETS['library_resummary_seconds'],
                    windows * window_prompt + final_prompt)})
    return scenarios


def verdict(scenarios, representative_hours):
    checks = [s['live_keeps_up'] and s['post_recording_within_target']
              and s['library_resummary_within_target']
              for s in scenarios if s['lecture_hours'] in representative_hours]
    if checks and all(checks):
        return 'feasible_estimate'
    if any(checks):
        return 'borderline_estimate'
    return 'infeasible_estimate'


def estimate_from_records(records, selected, tg_factor=1.0, a=ASSUMPTIONS):
    runtime, threads, fa = 'vulkan', selected['threads'], selected['flash_attn']
    tg = curve(records, 'tg', runtime, threads, fa)
    pp = prompt_curve(records, runtime, threads, fa, need_depth=needed_depth(a))
    scenarios = estimate(pp['points'], tg, a, tg_factor)
    return {'scope': 'estimate from raw throughput and assumed workload; not measured app latency',
            'selected': selected, 'targets': TARGETS, 'assumptions': a, 'tg_factor': tg_factor,
            'prompt_curve': pp, 'generation_curve': tg, 'scenarios': scenarios,
            'verdict': verdict(scenarios, a['representative_hours'])}


def server_factor(reports, tg):
    """Median ratio of schema-constrained server decode to llama-bench decode at the same depth."""
    ratios = []
    for report in reports:
        if report.get('response_format') != 'schema':
            raise ValueError('Server factor must come from schema-constrained runs')
        for run in report['runs']:
            t = run['timings']
            ratios.append(t['predicted_per_second']
                          / rate_at(tg, t['prompt_n'] + t['predicted_n'] / 2))
    if not ratios:
        raise ValueError('No server runs to derive a generation factor')
    return median(ratios)


def _number(value):
    return '-' if value is None else f'{value:g}'


def render_markdown(records, result):
    lines = ['| 런타임 | 스레드 | FA | ubatch | 시험 | 깊이 | 중앙값 tok/s | 범위 |',
             '| --- | --- | --- | --- | --- | --- | --- | --- |']
    for r in sorted(records, key=lambda r: (r['runtime'], r['threads'], r['flash_attn'],
                                            r['ubatch'], r['test'], r['depth'])):
        lines.append(f"| {r['runtime']} | {r['threads']} | {r['flash_attn']} | {r['ubatch']} "
                     f"| {r['test']}{r['tokens']} | {r['depth']} | {r['median_tps']:.2f} "
                     f"| {r['min_tps']:.2f}~{r['max_tps']:.2f} |")
    lines += ['', f"판정(추정): **{result['verdict']}**, 생성 보정 계수 {result['tg_factor']:.3f}", '',
              '| 강의(시간) | 음절/초 | 구간 처리(초) | 녹음 중 점유율 | 종료 후(초) | 다시 요약(초) '
              '| 필요 생성 속도 tok/s (종료 후 / 다시 요약) |',
              '| --- | --- | --- | --- | --- | --- | --- |']
    for s in result['scenarios']:
        lines.append(f"| {s['lecture_hours']} | {s['syllables_per_second']} | {s['window_seconds']} "
                     f"| {s['live_duty_ratio']} | {s['post_recording_seconds']} "
                     f"| {s['library_resummary_seconds']} | {_number(s['required_tg_tps_post'])} / "
                     f"{_number(s['required_tg_tps_resummary'])} |")
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('summary', help='llama-bench summary JSON from run_llama_bench.py')
    parser.add_argument('--server-report', nargs='*', default=[],
                        help='benchmark_llm.py report.json files run with --response-format schema')
    parser.add_argument('--output', help='write the estimate JSON here')
    parser.add_argument('--markdown', help='write Korean report tables to this UTF-8 file')
    args = parser.parse_args()
    summary = json.loads(Path(args.summary).read_text('utf-8'))
    records, selected = summary['records'], summary['selected']
    factor = 1.0
    if args.server_report:
        reports = [json.loads(Path(p).read_text('utf-8')) for p in args.server_report]
        factor = server_factor(reports, curve(records, 'tg', 'vulkan', selected['threads'],
                                              selected['flash_attn']))
    result = estimate_from_records(records, selected, factor)
    if args.output:
        Path(args.output).write_text(json.dumps(result, ensure_ascii=False, indent=2), 'utf-8')
    if args.markdown:
        # Files avoid console code-page mangling of Korean text on Windows.
        Path(args.markdown).write_text(render_markdown(records, result) + '\n', 'utf-8')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_feasibility.py -v`
Expected: PASS — `Ran 9 tests`, `OK`

- [ ] **Step 5: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 25 tests`, `OK`

```bash
git add scripts/feasibility.py tests/test_feasibility.py
git commit -m "feat: estimate lecture latency targets from throughput curves" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: llama-bench 측정 실행기

**Files:**
- Create: `scripts/run_llama_bench.py`
- Test: `tests/test_run_llama_bench.py`
- Modify: `evaluation/README.md` (`## 측정 범위` 바로 앞에 절 추가)

**Interfaces:**
- Consumes: Task 1의 `normalize`, `parse_jsonl`, `select_best`와 Task 2의 `estimate_from_records`, `render_markdown`.
- Produces:
  - `screen_jobs() -> list[dict]`, `depth_jobs(selected: dict) -> list[dict]`: 작업 항목은 `{'name', 'runtime', 'allow_failure', 'args'}`
  - `command(job, model_path, runtime_root, repetitions) -> list[str]`
  - `running_llama_processes(tasklist_csv: str) -> list[str]`, `describe_power(ac_line, battery_percent, status_flag) -> dict`, `power_status() -> dict`, `keep_awake()` 컨텍스트 관리자
  - `tail(path, lines=5) -> str`: 저장소 경로를 `<repo>`로 가린다.
  - `run_job(job, cmd, out_dir, timeout) -> tuple[dict, list[dict]]`, `unique_path(path) -> Path`, `overall_status(jobs) -> str`
  - 요약 JSON(`schema_version` 1): `kind`, `scope`, `runtime_tag`, `model_id`, `model_file`, `stage`, `repetitions`, `started_at`, `finished_at`, `power_at_start`, `power_at_end`, `selected`, `jobs`, `records`, `estimate`, `status`
  - 저장 위치: `evaluation/results/<날짜>-llama-bench-<stage>.json`(덮어쓰지 않음), `artifacts/llama-bench-<ns>/`(`summary.json`, `<job>.jsonl`, `<job>.log`, `estimate.md`)

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_run_llama_bench.py`:

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


class EnvironmentTests(unittest.TestCase):
    def test_detects_competing_inference_process(self):
        csv_text = ('"System","4","Services","0","132 K"\n'
                    '"llama-server.exe","1234","Console","1","5,000 K"\n')
        self.assertEqual(R.running_llama_processes(csv_text), ['llama-server.exe'])
        self.assertEqual(R.running_llama_processes('"python.exe","1","Console","1","1 K"\n'), [])

    def test_power_description(self):
        self.assertEqual(R.describe_power(1, 80, 0),
                         {'ac_power': True, 'battery_percent': 80, 'battery_saver': False})
        self.assertEqual(R.describe_power(0, 255, 1),
                         {'ac_power': False, 'battery_percent': None, 'battery_saver': True})

    def test_overall_status_distinguishes_expected_failures(self):
        ok = {'status': 'completed', 'allow_failure': False}
        risky = {'status': 'failed', 'allow_failure': True}
        broken = {'status': 'timeout', 'allow_failure': False}
        self.assertEqual(R.overall_status([ok]), 'completed')
        self.assertEqual(R.overall_status([ok, risky]), 'completed_with_expected_failures')
        self.assertEqual(R.overall_status([ok, risky, broken]), 'failed')

    def test_keep_awake_restores_normal_sleep(self):
        with patch.object(R.ctypes.windll.kernel32, 'SetThreadExecutionState') as api:
            with R.keep_awake():
                api.assert_called_once_with(0x80000001)
            api.assert_called_with(0x80000000)

    def test_unique_path_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'r.json'
            path.write_text('{}')
            self.assertEqual(R.unique_path(path).name, 'r-2.json')


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

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_run_llama_bench.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'run_llama_bench'`

- [ ] **Step 3: 최소 구현 작성**

`scripts/run_llama_bench.py`:

```python
"""Run the pinned llama-bench matrix for M0. Raw throughput only; see feasibility.py for estimates."""
import argparse
from contextlib import contextmanager
import csv
import ctypes
import io
import json
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

from feasibility import estimate_from_records, render_markdown
from llama_bench_results import normalize, parse_jsonl, select_best

ROOT = Path(__file__).resolve().parents[1]
BLOCKING_PROCESSES = {'llama-server.exe', 'llama-bench.exe', 'llama-cli.exe'}
JOB_TIMEOUT_SECONDS = 3 * 60 * 60
ES_CONTINUOUS, ES_SYSTEM_REQUIRED = 0x80000000, 0x00000001


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


def running_llama_processes(tasklist_csv):
    found = set()
    for row in csv.reader(io.StringIO(tasklist_csv)):
        if row and row[0].lower() in BLOCKING_PROCESSES:
            found.add(row[0].lower())
    return sorted(found)


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


@contextmanager
def keep_awake():
    """Block idle sleep while this process runs; system power settings stay unchanged."""
    kernel32 = ctypes.windll.kernel32
    kernel32.SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)
    try:
        yield
    finally:
        kernel32.SetThreadExecutionState(ES_CONTINUOUS)


def tail(path, lines=5):
    """Last log lines with the checkout path hidden, safe to commit in summaries."""
    text = path.read_text('utf-8', errors='replace') if path.exists() else ''
    text = '\n'.join(text.splitlines()[-lines:])
    for local in {str(ROOT), ROOT.as_posix()}:
        text = text.replace(local, '<repo>')
    return text


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


def unique_path(path):
    candidate, index = path, 2
    while candidate.exists():
        candidate = path.with_name(f'{path.stem}-{index}{path.suffix}')
        index += 1
    return candidate


def overall_status(jobs):
    failed = [j for j in jobs if j['status'] != 'completed']
    if not failed:
        return 'completed'
    if all(j['allow_failure'] for j in failed):
        return 'completed_with_expected_failures'
    return 'failed'


def now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds')


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
    tasks = subprocess.run(['tasklist', '/FO', 'CSV', '/NH'], capture_output=True, text=True,
                           encoding='oem', errors='replace', check=True).stdout
    if running := running_llama_processes(tasks):
        raise SystemExit(f"Stop other inference processes first: {', '.join(running)}")
    power = power_status()
    if power['ac_power'] is not True and not args.allow_battery:
        raise SystemExit(f'Connect AC power before benchmarking (power: {power})')

    out_dir = ROOT / 'artifacts' / f'llama-bench-{time.time_ns()}'
    out_dir.mkdir(parents=True)
    summary = {'schema_version': 1, 'kind': 'llama-bench-throughput',
               'scope': 'raw llama.cpp throughput on synthetic tokens; not app latency or quality',
               'runtime_tag': runtime['tag'], 'model_id': model['model_id'],
               'model_file': model['artifact']['filename'], 'stage': args.stage,
               'repetitions': args.repetitions, 'started_at': now(),
               'power_at_start': power, 'selected': None, 'jobs': [], 'records': []}

    def save():
        (out_dir / 'summary.json').write_text(
            json.dumps(summary, ensure_ascii=False, indent=2), 'utf-8')

    def run_all(jobs):
        for job in jobs:
            print(f"[{now()}] {job['name']} ...", flush=True)
            cmd = command(job, model_path, runtime_root, args.repetitions)
            result, records = run_job(job, cmd, out_dir, JOB_TIMEOUT_SECONDS)
            summary['jobs'].append(result)
            summary['records'].extend(records)
            save()
            print(f"[{now()}] {job['name']}: {result['status']} ({result['seconds']}s, "
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
    summary['finished_at'] = now()
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

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_run_llama_bench.py -v`
Expected: PASS — `Ran 11 tests`, `OK`

- [ ] **Step 5: 실제 바이너리로 인자 검증과 사전 점검 확인**

Run: `python scripts/run_llama_bench.py --stage depth`
Expected: 종료 코드 2, `error: --stage depth requires --threads and --flash-attn from a screen summary`

전체 측정은 Task 5에서만 실행한다. 이 단계에서는 오래 걸리는 측정을 시작하지 않는다.

- [ ] **Step 6: 사용법 문서화**

`evaluation/README.md`의 `## 측정 범위` 바로 앞에 다음 절을 추가한다.

````markdown
## llama-bench 처리량과 시간 목표 추정

```powershell
python scripts/run_llama_bench.py --stage all
python scripts/feasibility.py evaluation/results/<날짜>-llama-bench-all.json --server-report evaluation/results/<날짜>-constraint-schema.json --output evaluation/results/<날짜>-feasibility.json --markdown artifacts/<날짜>-feasibility.md
```

- `run_llama_bench.py`는 고정된 b10994의 `llama-bench`로 합성 토큰의 입력 처리(pp512)와 생성(tg128) 속도를 조건별 3회 측정한다. CPU 4·8스레드와 Vulkan 4·8스레드 × Flash Attention off·on을 깊이 0·2,048에서 비교한 뒤, 가장 빠른 생성 설정으로 깊이 0·2,048·8,192를 ubatch 128·512로 측정한다.
- ubatch 512의 8,192 깊이는 서버 시험에서 GPU device lost가 났던 조건이므로 실패를 허용하는 위험 시험으로 마지막에 실행한다. 실패해도 드라이버·TDR 설정을 바꾸지 않는다.
- AC 전원이 아니거나 다른 `llama-*` 프로세스가 실행 중이면 시작하지 않는다. 측정 중에는 이 프로세스만 절전을 막고 시스템 전원 설정은 바꾸지 않는다.
- 원출력·로그는 `artifacts/llama-bench-*/`, 요약은 `evaluation/results/<날짜>-llama-bench-<단계>.json`에 저장하며 기존 파일을 덮어쓰지 않는다. 요약에는 로컬 경로를 남기지 않는다.
- 오래 걸리는 측정은 `--stage screen`과 `--stage depth --threads <스레드> --flash-attn <on|off>`로 나눠 실행할 수 있다.
- `feasibility.py`는 측정 처리량과 결정 0006의 가정(발화 속도, 음절당 토큰, 구간 출력량 등)으로 녹음 종료 후 5분·다시 요약 1시간 목표를 추정한다. 결과는 추정이며 실제 앱 지연시간이나 요약 품질 통과가 아니다. 한국어 표는 인코딩 문제를 피하려고 `--markdown` 파일로만 쓴다.
````

- [ ] **Step 7: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 36 tests`, `OK`

```bash
git add scripts/run_llama_bench.py tests/test_run_llama_bench.py evaluation/README.md
git commit -m "feat: add pinned llama-bench matrix runner" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: 서버 출력 제약 비교 옵션

**Files:**
- Modify: `scripts/benchmark_llm.py`
- Test: `tests/test_benchmark_options.py`
- Modify: `evaluation/README.md` (Task 3에서 추가한 절의 끝)

**Interfaces:**
- Consumes: 기존 `summary_contract.schema_for_sources` 결과(`schema`).
- Produces:
  - `response_format_for(mode: str, strict: bool, schema: dict) -> dict | None`: `mode`는 `'auto'|'schema'|'json'|'none'`
  - CLI 옵션: `--threads N`(기본 8), `--response-format {auto,schema,json,none}`(기본 auto = 기존 동작)
  - 보고서 필드: `threads`, `response_format`(`'schema'|'json'|'none'`). Task 2의 `server_factor`가 `response_format == 'schema'`를 요구한다.

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_benchmark_options.py`:

```python
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import benchmark_llm as B  # noqa: E402


class ResponseFormatTests(unittest.TestCase):
    def test_auto_keeps_existing_contract_behavior(self):
        schema = {'type': 'object'}
        self.assertEqual(B.response_format_for('auto', True, schema),
                         {'type': 'json_object', 'schema': schema})
        self.assertEqual(B.response_format_for('auto', False, schema), {'type': 'json_object'})

    def test_explicit_modes(self):
        schema = {'type': 'object'}
        self.assertEqual(B.response_format_for('json', True, schema), {'type': 'json_object'})
        self.assertIsNone(B.response_format_for('none', True, schema))

    def test_unknown_mode_rejected(self):
        with self.assertRaises(ValueError):
            B.response_format_for('grammar', True, {})


if __name__ == '__main__':
    unittest.main()
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `python -m unittest discover -s tests -p test_benchmark_options.py -v`
Expected: FAIL — `AttributeError: module 'benchmark_llm' has no attribute 'response_format_for'`

- [ ] **Step 3: `benchmark_llm.py` 수정**

(a) `ROOT = Path(__file__).resolve().parents[1]` 다음, `def main():` 앞에 함수를 추가한다.

```python
def response_format_for(mode, strict, schema):
    """Server output constraint. 'none' exists only to isolate grammar cost in diagnostics."""
    if mode == 'auto':
        mode = 'schema' if strict else 'json'
    if mode == 'schema':
        return {'type': 'json_object', 'schema': schema}
    if mode == 'json':
        return {'type': 'json_object'}
    if mode == 'none':
        return None
    raise ValueError(f'Unknown response format: {mode}')
```

(b) `parser.add_argument('--ubatch-size', type=int, default=512)` 다음 줄에 옵션을 추가한다.

```python
    parser.add_argument('--threads', type=int, default=8)
    parser.add_argument('--response-format', choices=['auto', 'schema', 'json', 'none'],
                        default='auto', help='Diagnostics: vary only the server output constraint')
```

(c) `parser.error('Require 1 <= ubatch-size <= batch-size <= 16384')` 다음에 검증을 추가한다.

```python
    if args.threads < 1:
        parser.error('--threads must be positive')
    if args.response_format == 'schema' and not args.strict:
        parser.error('--response-format schema requires the strict contract prompt')
```

(d) `schema = schema_for_sources(segments)` 다음에 선택 결과를 계산한다.

```python
    response_format = response_format_for(args.response_format, args.strict, schema)
    format_name = ('none' if response_format is None
                   else 'schema' if 'schema' in response_format else 'json')
```

(e) 서버 명령의 스레드 인자를 바꾼다.

```python
# 변경 전
               '-ngl', '0' if args.backend == 'cpu' else '99', '-t', '8',
# 변경 후
               '-ngl', '0' if args.backend == 'cpu' else '99', '-t', str(args.threads),
```

(f) 보고서 초기값을 바꾼다.

```python
# 변경 전
              'context_tokens': 16384, 'threads': 8, 'runs': [], 'prefill_probes': [],
# 변경 후
              'context_tokens': 16384, 'threads': args.threads, 'runs': [], 'prefill_probes': [],
              'response_format': format_name,
```

(g) 요청 본문에서 고정된 `response_format` 줄을 제거하고 선택 결과를 조건부로 넣는다.

```python
# 변경 전
                    'seed': 42, 'max_tokens': 1536, 'cache_prompt': False,
                    'response_format': {'type': 'json_object', 'schema': schema} if args.strict else {'type': 'json_object'},
                    'reasoning_effort': 'none'}
# 변경 후
                    'seed': 42, 'max_tokens': 1536, 'cache_prompt': False,
                    'reasoning_effort': 'none'}
                if response_format is not None:
                    payload['response_format'] = response_format
```

strict 모드의 시스템 프롬프트(스키마 문자열 포함)와 계약 검증은 그대로 둔다. 따라서 세 모드는 프롬프트가 같고 서버 출력 제약만 다르다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `python -m unittest discover -s tests -p test_benchmark_options.py -v`
Expected: PASS — `Ran 3 tests`, `OK`

Run: `python scripts/benchmark_llm.py --backend vulkan --legacy-json --response-format schema`
Expected: 종료 코드 2, `error: --response-format schema requires the strict contract prompt`(서버를 시작하지 않음)

- [ ] **Step 5: 사용법 문서화**

Task 3에서 추가한 `evaluation/README.md` 절의 끝에 다음 문단을 추가한다.

```markdown
`benchmark_llm.py --response-format`은 같은 엄격한 프롬프트에서 서버 출력 제약만 바꾸는 진단 옵션이다. `schema`(strict 기본 동작), `json`, `none`의 생성 속도를 비교해 속도 저하가 문법 제약 때문인지 확인한다. `none`은 출력이 계약 검증에 실패할 수 있으며, 이때의 종료 코드 1은 예상된 결과다. `--threads`는 서버 CPU 스레드 수를 바꾼다. 보고서의 `response_format`과 `threads`에 실행 조건이 남는다.
```

- [ ] **Step 6: 전체 테스트 확인 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 39 tests`, `OK`

```bash
git add scripts/benchmark_llm.py tests/test_benchmark_options.py evaluation/README.md
git commit -m "feat: compare server output constraints in LLM benchmark" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: 측정 실행과 1차 판정 기록

**Files:**
- Create: `evaluation/results/<날짜>-llama-bench-all.json` (실행기가 생성)
- Create: `evaluation/results/<날짜>-constraint-none.json`, `-constraint-json.json`, `-constraint-schema.json`
- Create: `evaluation/results/<날짜>-feasibility.json`
- Create: `docs/validation/<날짜>-llama-bench.md`
- Modify: `docs/ROADMAP.md`, `docs/LLM-SPECIALIZATION.md`, `docs/decisions/0006-lecture-first-product-scope.md`, `README.md`

**Interfaces:**
- Consumes: Task 3의 요약 JSON, Task 4의 보고서 JSON, Task 2의 CLI.
- Produces: 판정 문자열(`feasible_estimate`·`borderline_estimate`·`infeasible_estimate`)과 그에 따른 로드맵 다음 작업.

- [ ] **Step 1: 사전 점검**

- 전원 어댑터를 연결하고 Samsung Settings의 전원 모드 이름을 적어 둔다(바꾸지 않는다).
- 브라우저·영상 등 무거운 프로그램을 종료하고, 측정이 끝날 때까지 노트북을 사용하지 않는다.

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 39 tests`, `OK`

Run: `tasklist /FI "IMAGENAME eq llama-server.exe"` 그리고 `tasklist /FI "IMAGENAME eq llama-bench.exe"`
Expected: 두 명령 모두 일치하는 작업이 없다는 메시지

- [ ] **Step 2: llama-bench 측정 실행 (약 1시간, 추정)**

Run: `python scripts/run_llama_bench.py --stage all`

진행 상황은 `artifacts/llama-bench-*/<작업 이름>.log`의 `llama-bench: benchmark x/y` 줄로 확인한다. 정상 종료 시 마지막 출력 형식은 다음과 같다(값은 예시).

```
Selected: {'threads': 8, 'flash_attn': 'on'}; status: completed_with_expected_failures
Verdict (raw throughput estimate): infeasible_estimate
Summary: C:\temp_git\Just-a-Click-\evaluation\results\2026-09-18-llama-bench-all.json
Logs: C:\temp_git\Just-a-Click-\artifacts\llama-bench-1789...
```

판단 규칙:
- `completed` 또는 `completed_with_expected_failures`(ubatch 512 위험 시험만 실패): Step 3으로 진행한다.
- `failed`: 요약의 `jobs[].error_tail`을 확인한다. 1분 뒤 Step 1의 프로세스 점검을 다시 하고, 실패한 단계만 **한 번** 재실행한다. screen이 실패했다면 `--stage screen`, depth가 실패했다면 `--stage depth --threads <Selected의 threads> --flash-attn <Selected의 flash_attn>`로 실행한다. 두 번째도 실패하면 드라이버·TDR을 건드리지 말고 Step 6의 보고서에 실패로 기록한 뒤, Step 9에서 사용자에게 보고한다.

- [ ] **Step 3: 서버 출력 제약 비교 (약 15분, 추정)**

Step 2의 `Selected` 값을 넣어 같은 설정으로 세 번 실행한다. 아래는 `threads=8`, `flash_attn=on`일 때의 예다.

```powershell
python scripts/benchmark_llm.py --backend vulkan --runs 3 --threads 8 --flash-attn on --response-format none
python scripts/benchmark_llm.py --backend vulkan --runs 3 --threads 8 --flash-attn on --response-format json
python scripts/benchmark_llm.py --backend vulkan --runs 3 --threads 8 --flash-attn on --response-format schema
```

Expected:
- `schema`: 각 실행 줄이 `errors=[]`이고 종료 코드 0이다. 합성 사례 회귀 검사만 실패하면 `completed_with_validation_failures`로 종료 코드 1이 나오지만, 속도 비교에는 사용하고 보고서에 적는다.
- `none`: 계약 검증 실패로 종료 코드 1이 나올 수 있다(예상된 결과).
- 각 실행은 마지막에 `Report: <디렉터리>`를 출력한다.

세 보고서를 결과 폴더로 복사한다. `<none 디렉터리>` 등은 각 실행이 출력한 `Report:` 경로다.

```powershell
Copy-Item <none 디렉터리>\report.json evaluation\results\<날짜>-constraint-none.json
Copy-Item <json 디렉터리>\report.json evaluation\results\<날짜>-constraint-json.json
Copy-Item <schema 디렉터리>\report.json evaluation\results\<날짜>-constraint-schema.json
```

`report.json`에는 인증 토큰·서버 명령·로컬 경로가 없으므로 그대로 커밋할 수 있다. `server.log`와 `response-*.json`은 복사하지 않는다.

- [ ] **Step 4: 출력 제약별 생성 속도 비교**

Run:

```powershell
python -c "import json,statistics,sys; [print(json.load(open(p,encoding='utf-8'))['response_format'], round(statistics.median(r['timings']['predicted_per_second'] for r in json.load(open(p,encoding='utf-8'))['runs']),3)) for p in sys.argv[1:]]" evaluation/results/<날짜>-constraint-none.json evaluation/results/<날짜>-constraint-json.json evaluation/results/<날짜>-constraint-schema.json
```

Expected: `none <값>`, `json <값>`, `schema <값>` 세 줄. 보고서에서는 다음 두 비율을 계산해 적는다.
- 문법 제약 영향 = schema ÷ none
- 서버·샘플링 영향 = none ÷ 요약 JSON의 Vulkan tg128 깊이 0 중앙값(Selected 설정)

- [ ] **Step 5: 서버 보정 계수를 반영한 추정**

Run:

```powershell
python scripts/feasibility.py evaluation/results/<날짜>-llama-bench-all.json --server-report evaluation/results/<날짜>-constraint-schema.json --output evaluation/results/<날짜>-feasibility.json --markdown artifacts/<날짜>-feasibility.md
```

Expected: 표준 출력의 JSON에 `"verdict"`와 `"tg_factor"`가 있고, `artifacts/<날짜>-feasibility.md`에 처리량 표와 시나리오 표가 생성된다. 보고서에는 이 추정(서버 보정 반영)을 최종 1차 판정으로 쓰고, Step 2의 원시 추정(`tg_factor` 1.0)은 참고로 함께 적는다.

- [ ] **Step 6: 측정 보고서 작성**

`docs/validation/<날짜>-llama-bench.md`를 다음 구조로 작성한다. 모든 수치는 명시한 파일에서 옮기며, 추정값에는 "추정"을 붙인다.

````markdown
# llama-bench 처리량과 시간 목표 1차 판정

- 날짜: <날짜>
- 모델·런타임: Qwen3-8B Q5_K_M, llama.cpp b10994 (CPU·Vulkan)
- 범위: 합성 토큰의 원시 처리량과 합성 회의 프롬프트의 서버 생성 속도. 실제 강의·STT·앱 지연시간·요약 품질 시험이 아님
- 결과 파일: [처리량](../../evaluation/results/<날짜>-llama-bench-all.json), [제약 비교: none](../../evaluation/results/<날짜>-constraint-none.json)·[json](../../evaluation/results/<날짜>-constraint-json.json)·[schema](../../evaluation/results/<날짜>-constraint-schema.json), [추정](../../evaluation/results/<날짜>-feasibility.json)

## 측정 조건

요약 JSON의 `power_at_start`·`power_at_end`, Step 1에서 적은 전원 모드, `repetitions`, 작업별 `status`·`seconds`를 표로 적는다. 실패한 작업은 `error_tail`의 핵심 줄을 인용한다. 온도·다른 부하는 통제하지 않았다고 명시한다.

## 처리량

`artifacts/<날짜>-feasibility.md`의 첫 번째 표를 붙여 넣는다. 선택된 설정(`selected`)과 선택 이유(깊이 0 생성 속도 우선)를 적는다. CPU 대비 Vulkan 배율, 스레드·FA 차이, 깊이 증가에 따른 감소 비율을 문장으로 요약한다.

## 출력 제약 비교

Step 4의 세 중앙값과 두 비율을 표로 적는다. 프롬프트는 같고 출력 제약만 다르다는 조건과 `none`의 계약 검증 결과를 함께 적는다.

## 시간 목표 추정

`artifacts/<날짜>-feasibility.md`의 판정 줄과 시나리오 표를 붙여 넣는다. 가정표(발화 속도, 음절당 토큰, 구간 출력량 등)를 옮기고, 실제 강의로 측정하지 않은 가정임을 적는다. 목표 달성에 필요한 생성 속도(`required_tg_tps_*`)와 현재 보정 후 생성 속도의 차이를 적는다.

## 판정과 다음 결정

1차 판정(추정), 판정 근거, 로드맵 반영 내용을 적는다. 출시 품질·앱 지연시간 판정이 아니라는 점을 적는다.

## 한계

합성 토큰·합성 회의 프롬프트, 1대의 기준 기기, 통제하지 않은 온도·부하, 가정 기반 작업량, STT 동시 실행 미반영, SYCL 미비교를 적는다.
````

- [ ] **Step 7: 판정에 따라 문서 갱신**

`docs/ROADMAP.md`:
- M0 남은 작업 1을 `[x]`로 바꾸고, 끝에 `결과: <판정>(추정), [보고서](validation/<날짜>-llama-bench.md).`를 붙인다.
- `## 7. 바로 다음 작업`을 판정에 따라 교체한다.
  - `feasible_estimate`: **남은 작업 2(STT 후보 측정)와 3(동시 실행 측정)**을 다음으로 둔다. STT가 GPU를 함께 쓰면 여유가 줄어든다는 점을 적는다.
  - `borderline_estimate`: 목표를 넘은 시나리오를 적고, **구간 크기·출력량 조정 실험과 STT 측정**을 다음으로 둔다.
  - `infeasible_estimate`: **사용자 결정 요청**을 다음 작업으로 둔다. 선택지(시간 목표 조정, 구간 출력량 축소, 가속 경로 비교, 모델 재검토 결정)를 필요 생성 속도와 함께 나열하고, 결정 전에는 모델·목표를 바꾸지 않는다고 적는다.

`docs/LLM-SPECIALIZATION.md` `## 7. 시간 목표의 병목 관리`의 "M0 실측 참고값" 문단 끝에 다음 문장을 붙인다.
`llama-bench 측정([보고서](validation/<날짜>-llama-bench.md))에서 스키마 제약 없는 Vulkan 생성은 깊이 0 기준 <값> tok/s, 서버 스키마 제약 보정 계수는 <값>이었고 1차 판정은 <판정>(추정)이다.`

`docs/decisions/0006-lecture-first-product-scope.md` `## 5. 위험과 후속 검증`의 첫 항목 끝에 다음 문장을 붙인다.
`1차 측정 결과는 [보고서](../validation/<날짜>-llama-bench.md)에 기록했다(판정: <판정>, 추정).`

`README.md`의 문서 목록에서 `- [회의 스키마·긴 입력 처리 진단](...)` 다음 줄에 추가한다.
`- [llama-bench 처리량과 시간 목표 1차 판정](docs/validation/<날짜>-llama-bench.md)`

- [ ] **Step 8: 검증 후 커밋**

Run: `python -m unittest discover -s tests -v`
Expected: `Ran 39 tests`, `OK`

Run:

```bash
python - <<'EOF'
import json, re, pathlib
root = pathlib.Path('.')
for p in root.glob('evaluation/results/*.json'):
    json.loads(p.read_text('utf-8'))
bad = []
for md in list(root.glob('*.md')) + list(root.glob('docs/**/*.md')) + list(root.glob('evaluation/*.md')):
    for t in re.findall(r'\]\(([^)\s]+)\)', md.read_text('utf-8')):
        if not t.startswith(('http', '#')) and '<' not in t and not (md.parent / t.split('#')[0]).resolve().exists():
            bad.append((str(md), t))
print('broken links:', bad or 'none')
EOF
```

Expected: `broken links: none` (`evaluation/README.md`의 `<날짜>` 예시 경로는 링크가 아니므로 검사 대상이 아니다)

Run: `git status --short`
Expected: 이번 Task의 결과·문서 파일만 변경되어 있고 `artifacts/`는 표시되지 않는다.

```bash
git add evaluation/results/<날짜>-llama-bench-all.json evaluation/results/<날짜>-constraint-none.json evaluation/results/<날짜>-constraint-json.json evaluation/results/<날짜>-constraint-schema.json evaluation/results/<날짜>-feasibility.json docs/validation/<날짜>-llama-bench.md docs/ROADMAP.md docs/LLM-SPECIALIZATION.md docs/decisions/0006-lecture-first-product-scope.md README.md
git commit -m "test: measure llama-bench throughput and estimate latency targets" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 9: 사용자 보고와 결정 요청**

작업·결과·다음 형식으로 보고한다. 결과에는 선택된 설정, Vulkan 생성 중앙값(깊이 0·8,192), 출력 제약 비율, 1차 판정(추정)과 필요 생성 속도를 포함한다. 판정이 `infeasible_estimate`이면 Step 7의 선택지를 제시하고 사용자 결정을 기다린다. 모델·시간 목표·처리 방식은 결정 없이 바꾸지 않는다.
