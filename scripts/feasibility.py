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
