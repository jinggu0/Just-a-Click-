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
