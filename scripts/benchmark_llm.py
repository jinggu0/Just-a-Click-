"""M0 synthetic smoke evaluation; does not establish real-world summary quality."""
import argparse
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--backend', choices=['cpu', 'vulkan'], required=True)
    parser.add_argument('--runs', type=int, default=3)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error('--runs must be positive')
    model = json.loads((ROOT / 'config/llm-model.json').read_text('utf-8'))
    runtime = json.loads((ROOT / 'config/llama-runtime.json').read_text('utf-8'))
    fixture = json.loads((ROOT / 'evaluation/fixtures/meeting-smoke.json').read_text('utf-8'))
    executable = ROOT / 'runtimes' / runtime['tag'] / args.backend / 'llama-server.exe'
    out = ROOT / 'artifacts' / f'smoke-{args.backend}-{time.time_ns()}'
    out.mkdir(parents=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    key = secrets.token_hex(32)
    env = os.environ.copy()
    env['LLAMA_API_KEY'] = key
    command = [str(executable), '-m', str(ROOT / 'models' / model['artifact']['filename']),
               '--host', '127.0.0.1', '--port', str(port), '-c', '16384', '-np', '1',
               '-ngl', '0' if args.backend == 'cpu' else '99', '-t', '8',
               '--jinja', '--chat-template-kwargs', '{"enable_thinking":false}']
    report = {'backend': args.backend, 'runtime_tag': runtime['tag'],
              'model_id': model['model_id'], 'fixture_id': fixture['id'],
              'context_tokens': 16384, 'threads': 8, 'runs': [],
              'scope': 'synthetic short transcript; not four-hour or STT validation'}
    started = time.perf_counter()
    process = None
    # Ignore proxy environment settings for all loopback requests.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with (out / 'server.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen(command, stdout=log, stderr=log, env=env,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
            while True:
                if process.poll() is not None:
                    raise RuntimeError(f'Server exited {process.returncode}; see {out}')
                if time.perf_counter() - started > 240:
                    raise TimeoutError('Model startup exceeded 240 seconds')
                try:
                    request = urllib.request.Request(f'http://127.0.0.1:{port}/health',
                                                     headers={'Authorization': f'Bearer {key}'})
                    with opener.open(request, timeout=2) as response:
                        if response.status == 200:
                            break
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(0.5)
            report['startup_seconds'] = time.perf_counter() - started
            print(f"{args.backend}: ready after {report['startup_seconds']:.2f}s", flush=True)
            for run in range(args.runs):
                payload = {
                    'messages': [{'role': 'system', 'content': fixture['system']},
                                 {'role': 'user', 'content': fixture['transcript']}],
                    'temperature': 0.7, 'top_p': 0.8, 'top_k': 20, 'min_p': 0,
                    'seed': 42, 'max_tokens': 1536, 'cache_prompt': False,
                    'response_format': {'type': 'json_object'},
                    'chat_template_kwargs': {'enable_thinking': False}}
                tick = time.perf_counter()
                request = urllib.request.Request(f'http://127.0.0.1:{port}/v1/chat/completions',
                    data=json.dumps(payload, ensure_ascii=False).encode('utf-8'),
                    headers={'Content-Type': 'application/json', 'Authorization': f'Bearer {key}'})
                with opener.open(request, timeout=600) as response:
                    result = json.load(response)
                (out / f'response-{run + 1}.json').write_text(
                    json.dumps(result, ensure_ascii=False, indent=2), 'utf-8')
                elapsed = time.perf_counter() - tick
                choice = result['choices'][0]
                content = choice['message'].get('content', '')
                try:
                    parsed = json.loads(content)
                    valid = isinstance(parsed, dict) and all(
                        field in parsed for field in ['title', 'decisions', 'actions', 'open_questions'])
                except (ValueError, TypeError):
                    valid = False
                report['runs'].append({'run': run + 1, 'request_to_saved_seconds': elapsed,
                    'startup_plus_request_seconds': report['startup_seconds'] + elapsed if run == 0 else None,
                    'finish_reason': choice.get('finish_reason'), 'required_json_fields': valid,
                    'reasoning_present': bool(choice['message'].get('reasoning_content')) or '<think>' in content,
                    'usage': result.get('usage'), 'timings': result.get('timings')})
                print(f'{args.backend} run {run + 1}: {elapsed:.2f}s; JSON fields={valid}', flush=True)
            report['status'] = 'completed'
    except Exception as error:
        report['status'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        (out / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), 'utf-8')
        print(f'Report: {out}', flush=True)


if __name__ == '__main__':
    main()
