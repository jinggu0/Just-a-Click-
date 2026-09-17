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
