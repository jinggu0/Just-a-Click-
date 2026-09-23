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
# llama-server keeps every finished request's state in a prompt cache (8 GiB by default),
# which grew the process by ~180 MiB per draft, so benchmarks run with it disabled.
CACHE_RAM_MIB = 0
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
    parser.add_argument('--cache-ram', type=int, default=CACHE_RAM_MIB,
                        help='llama-server prompt cache in MiB; 0 disables it')
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
               'llm_context_tokens': args.context_tokens, 'llm_cache_ram_mib': args.cache_ram,
               'fixture_id': FIXTURE_ID,
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
                             threads=4, cache_ram=args.cache_ram) as server:
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
                         context_tokens=args.context_tokens, threads=llm_threads,
                         cache_ram=args.cache_ram) as server:
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
