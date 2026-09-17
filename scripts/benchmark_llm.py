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
from summary_contract import (STRICT_INSTRUCTION, parse_segments, schema_for_sources,
                              validate_summary, check_smoke_expectations)

ROOT = Path(__file__).resolve().parents[1]


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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--backend', choices=['cpu', 'vulkan'], required=True)
    parser.add_argument('--runs', type=int, default=3)
    contract_mode = parser.add_mutually_exclusive_group()
    contract_mode.add_argument('--strict', dest='strict', action='store_true', default=True)
    contract_mode.add_argument('--legacy-json', dest='strict', action='store_false',
                               help='Comparison only: reproduce the old weak JSON object contract')
    parser.add_argument('--flash-attn', choices=['auto', 'on', 'off'], default='auto')
    parser.add_argument('--prefill-tokens', nargs='*', type=int, default=[])
    parser.add_argument('--probe-only', action='store_true')
    parser.add_argument('--batch-size', type=int, default=2048)
    parser.add_argument('--ubatch-size', type=int, default=512)
    parser.add_argument('--threads', type=int, default=8)
    parser.add_argument('--response-format', choices=['auto', 'schema', 'json', 'none'],
                        default='auto', help='Diagnostics: vary only the server output constraint')
    args = parser.parse_args()
    if args.runs < 1:
        parser.error('--runs must be positive')
    if any(n < 1 or n > 16380 for n in args.prefill_tokens):
        parser.error('Prefill probes must fit 1..16380 tokens without context shifting')
    if args.probe_only and not args.prefill_tokens:
        parser.error('--probe-only requires --prefill-tokens')
    if not 1 <= args.ubatch_size <= args.batch_size <= 16384:
        parser.error('Require 1 <= ubatch-size <= batch-size <= 16384')
    if args.threads < 1:
        parser.error('--threads must be positive')
    if args.response_format == 'schema' and not args.strict:
        parser.error('--response-format schema requires the strict contract prompt')
    model = json.loads((ROOT / 'config/llm-model.json').read_text('utf-8'))
    runtime = json.loads((ROOT / 'config/llama-runtime.json').read_text('utf-8'))
    fixture = json.loads((ROOT / 'evaluation/fixtures/meeting-smoke.json').read_text('utf-8'))
    segments = parse_segments(fixture['transcript'])
    schema = schema_for_sources(segments)
    response_format = response_format_for(args.response_format, args.strict, schema)
    format_name = ('none' if response_format is None
                   else 'schema' if 'schema' in response_format else 'json')
    system = fixture['system']
    if args.strict:
        system += STRICT_INSTRUCTION + '\nJSON 스키마: ' + json.dumps(schema, ensure_ascii=False, separators=(',', ':'))
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
               '-ngl', '0' if args.backend == 'cpu' else '99', '-t', str(args.threads),
               '--jinja', '--reasoning', 'off', '--flash-attn', args.flash_attn,
               '-b', str(args.batch_size), '-ub', str(args.ubatch_size)]
    report = {'backend': args.backend, 'runtime_tag': runtime['tag'],
              'model_id': model['model_id'], 'fixture_id': fixture['id'],
              'context_tokens': 16384, 'threads': args.threads, 'runs': [], 'prefill_probes': [],
              'response_format': format_name,
              'strict_contract': args.strict, 'flash_attention_requested': args.flash_attn,
              'evaluator_version': 2, 'reasoning': 'off',
              'batch_size': args.batch_size, 'ubatch_size': args.ubatch_size, 'probe_only': args.probe_only,
              'scope': 'synthetic short transcript; not four-hour or STT validation'}
    started = time.perf_counter()
    process = None
    # Ignore proxy environment settings for all loopback requests.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def post(endpoint, payload):
        request = urllib.request.Request(f'http://127.0.0.1:{port}{endpoint}',
            data=json.dumps(payload, ensure_ascii=False).encode('utf-8'),
            headers={'Content-Type': 'application/json', 'Authorization': f'Bearer {key}'})
        try:
            with opener.open(request, timeout=600) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            details = error.read(4096).decode('utf-8', errors='replace')
            raise RuntimeError(f'HTTP {error.code}: {details}') from error

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
            for run in range(0 if args.probe_only else args.runs):
                payload = {
                    'messages': [{'role': 'system', 'content': system},
                                 {'role': 'user', 'content': fixture['transcript']}],
                    'temperature': 0.7, 'top_p': 0.8, 'top_k': 20, 'min_p': 0,
                    'seed': 42, 'max_tokens': 1536, 'cache_prompt': False,
                    'reasoning_effort': 'none'}
                if response_format is not None:
                    payload['response_format'] = response_format
                tick = time.perf_counter()
                template = post('/apply-template', {'messages': payload['messages']})['prompt']
                input_tokens = len(post('/tokenize', {'content': template, 'add_special': False})['tokens'])
                if input_tokens + payload['max_tokens'] > 16384:
                    raise ValueError('Input plus reserved output exceeds context; chunking is required')
                result = post('/v1/chat/completions', payload)
                (out / f'response-{run + 1}.json').write_text(
                    json.dumps(result, ensure_ascii=False, indent=2), 'utf-8')
                choice = result['choices'][0]
                content = choice['message'].get('content', '')
                try:
                    parsed = json.loads(content)
                    valid = isinstance(parsed, dict) and all(
                        field in parsed for field in ['title', 'decisions', 'actions', 'open_questions'])
                except (ValueError, TypeError):
                    valid = False
                contract_errors = []
                checks = None
                if args.strict:
                    try:
                        if choice['message'].get('reasoning_content') or '<think>' in content:
                            raise ValueError('Unexpected reasoning output')
                        parsed = validate_summary(content, segments, choice.get('finish_reason'))
                        checks = check_smoke_expectations(parsed)
                        if not all(checks.values()):
                            contract_errors.append('Fixture regression checks failed')
                        if not contract_errors:
                            (out / f'accepted-{run + 1}.json').write_text(
                                json.dumps(parsed, ensure_ascii=False, separators=(',', ':')), 'utf-8')
                    except (ValueError, TypeError) as error:
                        contract_errors.append(str(error))
                elapsed = time.perf_counter() - tick
                report['runs'].append({'run': run + 1, 'request_to_saved_seconds': elapsed,
                    'startup_plus_request_seconds': report['startup_seconds'] + elapsed if run == 0 else None,
                    'finish_reason': choice.get('finish_reason'), 'required_json_fields': valid,
                    'reasoning_present': bool(choice['message'].get('reasoning_content')) or '<think>' in content,
                    'preflight_input_tokens': input_tokens,
                    'contract_errors': contract_errors, 'fixture_checks': checks,
                    'usage': result.get('usage'), 'timings': result.get('timings')})
                print(f'{args.backend} run {run + 1}: {elapsed:.2f}s; JSON fields={valid}; errors={contract_errors}', flush=True)
            for count in args.prefill_tokens:
                # Repeated synthetic input isolates prefill cost, NOT long-document comprehension.
                corpus = (fixture['transcript'] + '\n') * (count // 100 + 2)
                tokens = post('/tokenize', {'content': corpus, 'add_special': False, 'parse_special': False})['tokens']
                if len(tokens) < count:
                    raise ValueError('Insufficient synthetic probe tokens')
                tick = time.perf_counter()
                probe = {'requested_tokens': count, 'status': 'running',
                         'scope': 'input throughput only; one generated token'}
                report['prefill_probes'].append(probe)
                try:
                    result = post('/completion', {'prompt': tokens[:count], 'n_predict': 1,
                        'cache_prompt': False, 'temperature': 0, 'seed': 42})
                except Exception as error:
                    probe.update(status='failed', error=str(error),
                                 request_seconds=time.perf_counter() - tick)
                    raise
                (out / f'prefill-{count}.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), 'utf-8')
                measured_count = result.get('timings', {}).get('prompt_n')
                passed = result.get('truncated') is False and measured_count == count
                probe.update({'status': 'completed' if passed else 'failed',
                    'request_to_saved_seconds': time.perf_counter() - tick,
                    'truncated': result.get('truncated'), 'token_count_passed': passed,
                    'timings': result.get('timings')})
                print(f'{args.backend} prefill {count}: {report["prefill_probes"][-1]["request_to_saved_seconds"]:.2f}s; counts={passed}', flush=True)
                if not passed:
                    raise ValueError('Prefill probe truncated or did not process all requested tokens')
            report['status'] = ('completed_with_validation_failures'
                if any(run['contract_errors'] for run in report['runs']) else 'completed')
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
    if report['status'] != 'completed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
