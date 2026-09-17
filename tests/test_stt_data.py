import array
import io
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest
import wave

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import stt_data as D  # noqa: E402


def float_wav(values, rate=16000, channels=1):
    payload = array.array('f', values).tobytes()
    fmt = struct.pack('<HHIIHH', 3, channels, rate, rate * 4 * channels, 4 * channels, 32)
    body = (b'WAVE' + b'fmt ' + struct.pack('<I', len(fmt)) + fmt
            + b'fact' + struct.pack('<I', 4) + struct.pack('<I', len(values))
            + b'data' + struct.pack('<I', len(payload)) + payload)
    return b'RIFF' + struct.pack('<I', len(body)) + body


def pcm_wav(values):
    buffer = io.BytesIO()
    with wave.open(buffer, 'wb') as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(16000)
        audio.writeframes(array.array('h', values).tobytes())
    return buffer.getvalue()


def row(sentence_id, name, samples, text):
    return f'{sentence_id}\t{name}\tRAW {text}\t{text}\tx | y\t{samples}\tMALE'


class TsvTests(unittest.TestCase):
    def test_parse_and_select_first_recording_per_sentence(self):
        text = '\n'.join([row(10, 'b.wav', 5, '열'), row(9, 'z.wav', 5, '아홉'),
                          row(10, 'a.wav', 6, '십')]) + '\n'
        chosen = D.select_utterances(D.parse_tsv(text))
        self.assertEqual([(u['sentence_id'], u['file_name']) for u in chosen],
                         [(9, 'z.wav'), (10, 'a.wav')])
        self.assertEqual(chosen[1]['transcription'], '십')

    def test_malformed_row_rejected(self):
        with self.assertRaises(ValueError):
            D.parse_tsv('1\ta.wav\tonly three')


class ChunkPlanTests(unittest.TestCase):
    def utterance(self, sentence_id, samples):
        return {'sentence_id': sentence_id, 'file_name': f'{sentence_id}.wav',
                'samples': samples, 'transcription': f't{sentence_id}'}

    def test_packs_within_limit_with_gaps_and_skips_long_items(self):
        items = [self.utterance(i, n) for i, n in enumerate([100, 100, 100, 350, 50, 60], 1)]
        chunks = D.plan_chunks(items, limit_seconds=30, gap_seconds=0.5, count=5, rate=10)
        self.assertEqual([[u['sentence_id'] for u in c['utterances']] for c in chunks],
                         [[1, 2], [3, 5, 6]])
        self.assertEqual([c['samples'] for c in chunks], [205, 220])
        self.assertEqual(chunks[1]['reference'], 't3 t5 t6')
        self.assertEqual((chunks[0]['chunk_id'], chunks[0]['seconds']), ('fleurs-ko-001', 20.5))

    def test_stops_at_requested_count(self):
        items = [self.utterance(i, 200) for i in range(1, 6)]
        chunks = D.plan_chunks(items, limit_seconds=30, gap_seconds=0.5, count=2, rate=10)
        self.assertEqual(len(chunks), 2)
        self.assertEqual(chunks[-1]['utterances'][0]['sentence_id'], 2)


class WavTests(unittest.TestCase):
    def test_reads_float_and_pcm16(self):
        self.assertEqual(list(D.read_wav_samples(float_wav([0.5, -0.25]))), [0.5, -0.25])
        self.assertEqual(list(D.read_wav_samples(pcm_wav([16384, -32768]))), [0.5, -1.0])

    def test_rejects_unexpected_audio(self):
        with self.assertRaises(ValueError):
            D.read_wav_samples(float_wav([0.0, 0.0], channels=2))
        with self.assertRaises(ValueError):
            D.read_wav_samples(float_wav([0.0], rate=48000))
        with self.assertRaises(ValueError):
            D.read_wav_samples(b'not a wav file')

    def test_pcm16_conversion_clips(self):
        self.assertEqual(list(D.to_pcm16([0.5, 1.5, -2.0, 0.0])), [16384, 32767, -32768, 0])


class BuildTests(unittest.TestCase):
    def test_build_writes_verifiable_chunks(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            tsv = base / 'test.tsv'
            tsv.write_text(row(2, 'b.wav', 1600, '둘') + '\n' + row(1, 'a.wav', 1600, '하나') + '\n',
                           'utf-8')
            tar_path = base / 'test.tar.gz'
            with tarfile.open(tar_path, 'w:gz') as tar:
                for name, value in (('b.wav', -0.5), ('a.wav', 0.5)):
                    data = float_wav([value] * 1600)
                    info = tarfile.TarInfo(f'test/{name}')
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
            out = base / 'chunks'
            fixture = D.build(tsv, tar_path, out, {'dataset': 'google/fleurs'}, count=1)
            chunk = fixture['chunks'][0]
            self.assertEqual(chunk['samples'], 1600 + 8000 + 1600)
            self.assertEqual(chunk['reference'], '하나 둘')
            with wave.open(str(out / 'fleurs-ko-001.wav')) as audio:
                frames = array.array('h', audio.readframes(audio.getnframes()))
            self.assertEqual((frames[0], frames[1600], frames[-1]), (16384, 0, -16384))
            self.assertEqual(D.verify_chunks(fixture, out), [])
            (out / 'fleurs-ko-001.wav').unlink()
            self.assertEqual(D.verify_chunks(fixture, out), ['missing fleurs-ko-001'])

    def test_build_rejects_sample_count_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            tsv = base / 'test.tsv'
            tsv.write_text(row(1, 'a.wav', 999, '하나') + '\n', 'utf-8')
            tar_path = base / 'test.tar.gz'
            with tarfile.open(tar_path, 'w:gz') as tar:
                data = float_wav([0.1] * 10)
                info = tarfile.TarInfo('test/a.wav')
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
            with self.assertRaisesRegex(ValueError, 'Sample count mismatch'):
                D.build(tsv, tar_path, base / 'out', {}, count=1)


if __name__ == '__main__':
    unittest.main()
