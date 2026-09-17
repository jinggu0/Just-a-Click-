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
