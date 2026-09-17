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
