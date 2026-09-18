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


def run_folder(out_dir, stage, model_id, build, threads, timestamps):
    """Raw output folder per stage: the screening model repeats in the model comparison."""
    return out_dir / f"{stage}-{model_id}-{build}-t{threads}-{'ts' if timestamps else 'nt'}"


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

    def run_set(stage, build, threads, model_id, chunk_list, timestamps=False):
        entry = {'model': model_id, 'build': build, 'threads': threads,
                 'timestamps': timestamps, 'decoding': None, 'records': []}
        summary[stage].append(entry)
        label = f"{model_id} {build} t{threads} {'ts' if timestamps else 'nt'}"
        folder = run_folder(out_dir, stage, model_id, build, threads, timestamps)
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
                run_set('screen', config['build'], config['threads'],
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
                run_set('models', config['build'], config['threads'], model_id, chunks)
            summary['selected_model'] = select_model(summary['models'])
            for model_id in top_models(summary['models']):
                run_set('timestamp_check', config['build'], config['threads'], model_id,
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
