"""Build the FLEURS Korean 30-second chunk set. Public CC BY 4.0 read speech, not lecture audio."""
import array
import json
from pathlib import Path, PurePosixPath
import struct
import tarfile
import wave

from prepare_llm import verify

ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ID = 'stt-fleurs-ko-v1'
SAMPLE_RATE = 16000
CHUNK_LIMIT_SECONDS = 30.0
GAP_SECONDS = 0.5
CHUNK_COUNT = 60
SELECTION = {'recording': 'one per sentence id, lexicographically first file name',
             'order': 'sentence id ascending', 'gap_seconds': GAP_SECONDS,
             'max_chunk_seconds': CHUNK_LIMIT_SECONDS, 'chunks': CHUNK_COUNT,
             'reference': 'FLEURS normalized transcription (4th column) joined by spaces'}


def chunk_dir(fixture_id=FIXTURE_ID):
    return ROOT / 'artifacts/stt-chunks' / fixture_id


def parse_tsv(text):
    rows = []
    for line in text.splitlines():
        if not line.strip():
            continue
        fields = line.split('\t')
        if len(fields) != 7:
            raise ValueError('FLEURS rows must have 7 tab-separated fields')
        sentence_id, file_name, raw, normalized, _, samples, gender = fields
        rows.append({'sentence_id': int(sentence_id), 'file_name': file_name,
                     'raw_transcription': raw, 'transcription': normalized,
                     'samples': int(samples), 'gender': gender})
    return rows


def select_utterances(rows):
    """One recording per sentence (lexicographically first file name), ordered by sentence id."""
    chosen = {}
    for row in rows:
        best = chosen.get(row['sentence_id'])
        if best is None or row['file_name'] < best['file_name']:
            chosen[row['sentence_id']] = row
    return [chosen[key] for key in sorted(chosen)]


def chunk_record(index, group, gap, rate):
    samples = sum(u['samples'] for u in group) + gap * (len(group) - 1)
    return {'chunk_id': f'fleurs-ko-{index:03d}', 'samples': samples,
            'seconds': round(samples / rate, 3),
            'utterances': [{'sentence_id': u['sentence_id'], 'file_name': u['file_name'],
                            'samples': u['samples']} for u in group],
            'reference': ' '.join(u['transcription'] for u in group)}


def plan_chunks(utterances, limit_seconds=CHUNK_LIMIT_SECONDS, gap_seconds=GAP_SECONDS,
                count=CHUNK_COUNT, rate=SAMPLE_RATE):
    limit, gap = round(limit_seconds * rate), round(gap_seconds * rate)
    groups, current, length = [], [], 0
    for utterance in utterances:
        if utterance['samples'] > limit:
            continue
        added = utterance['samples'] + (gap if current else 0)
        if current and length + added > limit:
            groups.append(current)
            if len(groups) == count:
                break
            current, length, added = [], 0, utterance['samples']
        current.append(utterance)
        length += added
    else:
        if current:
            groups.append(current)
    return [chunk_record(i, group, gap, rate) for i, group in enumerate(groups[:count], 1)]


def read_wav_samples(data):
    """Mono 16 kHz samples in [-1, 1] from PCM16 or IEEE float32 RIFF bytes (little-endian)."""
    if data[:4] != b'RIFF' or data[8:12] != b'WAVE':
        raise ValueError('Not a RIFF/WAVE file')
    position, fmt, payload = 12, None, None
    while position + 8 <= len(data):
        chunk_id = data[position:position + 4]
        size = struct.unpack('<I', data[position + 4:position + 8])[0]
        body = data[position + 8:position + 8 + size]
        if chunk_id == b'fmt ':
            fmt = struct.unpack('<HHIIHH', body[:16])
        elif chunk_id == b'data':
            payload = body
        position += 8 + size + (size & 1)
    if fmt is None or payload is None:
        raise ValueError('WAV is missing a fmt or data chunk')
    tag, channels, rate, _, _, bits = fmt
    if channels != 1 or rate != SAMPLE_RATE:
        raise ValueError('Expected mono 16 kHz audio')
    if tag == 3 and bits == 32:
        samples = array.array('f')
        samples.frombytes(payload[:len(payload) // 4 * 4])
        return samples
    if tag == 1 and bits == 16:
        ints = array.array('h')
        ints.frombytes(payload[:len(payload) // 2 * 2])
        return array.array('f', (value / 32768 for value in ints))
    raise ValueError(f'Unsupported WAV encoding: format {tag}, {bits} bits')


def to_pcm16(samples):
    return array.array('h', (max(-32768, min(32767, round(s * 32767))) for s in samples))


def write_wav(path, pcm):
    with wave.open(str(path), 'wb') as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(SAMPLE_RATE)
        audio.writeframes(pcm.tobytes())


def build(tsv_path, tar_path, out_dir, source, count=CHUNK_COUNT):
    chunks = plan_chunks(select_utterances(parse_tsv(tsv_path.read_text('utf-8'))), count=count)
    if len(chunks) != count:
        raise ValueError(f'Only {len(chunks)} of {count} chunks could be built')
    needed = {u['file_name']: u['samples'] for chunk in chunks for u in chunk['utterances']}
    audio = {}
    with tarfile.open(tar_path) as tar:
        # One sequential pass: seeking backwards in a gzip tar decompresses from the start.
        for member in tar:
            name = PurePosixPath(member.name).name
            if member.isfile() and name in needed:
                samples = read_wav_samples(tar.extractfile(member).read())
                if len(samples) != needed[name]:
                    raise ValueError(f'Sample count mismatch for {name}')
                audio[name] = to_pcm16(samples)
    if missing := sorted(set(needed) - set(audio)):
        raise ValueError(f'Audio missing from archive: {missing[:3]}')
    out_dir.mkdir(parents=True, exist_ok=True)
    gap = array.array('h', bytes(2 * round(GAP_SECONDS * SAMPLE_RATE)))
    for chunk in chunks:
        pcm = array.array('h')
        for index, utterance in enumerate(chunk['utterances']):
            if index:
                pcm.extend(gap)
            pcm.extend(audio[utterance['file_name']])
        write_wav(out_dir / f"{chunk['chunk_id']}.wav", pcm)
    return {'schema_version': 1, 'fixture_id': FIXTURE_ID, 'source': source,
            'selection': dict(SELECTION, chunks=count), 'sample_rate': SAMPLE_RATE,
            'chunks': chunks}


def verify_chunks(fixture, directory):
    problems = []
    for chunk in fixture['chunks']:
        path = directory / f"{chunk['chunk_id']}.wav"
        if not path.exists():
            problems.append(f"missing {chunk['chunk_id']}")
            continue
        with wave.open(str(path)) as audio:
            shape = (audio.getnchannels(), audio.getframerate(), audio.getsampwidth(),
                     audio.getnframes())
        if shape != (1, SAMPLE_RATE, 2, chunk['samples']):
            problems.append(f"invalid {chunk['chunk_id']}")
    return problems


def main():
    source = json.loads((ROOT / 'config/stt-eval-data.json').read_text('utf-8'))
    folder = ROOT / 'downloads/fleurs' / source['config']
    for item in source['files']:
        if not verify(folder / PurePosixPath(item['path']).name, item['size_bytes'], item['sha256']):
            raise SystemExit('FLEURS files missing or corrupt; run scripts/prepare_stt.py first')
    fixture = build(folder / 'test.tsv', folder / 'test.tar.gz', chunk_dir(), source)
    path = ROOT / 'evaluation/fixtures' / f'{FIXTURE_ID}.json'
    path.write_text(json.dumps(fixture, ensure_ascii=False, indent=2) + '\n', 'utf-8')
    total = sum(chunk['seconds'] for chunk in fixture['chunks'])
    print(f"Wrote {len(fixture['chunks'])} chunks ({total:.1f}s) and {path.name}", flush=True)


if __name__ == '__main__':
    main()
