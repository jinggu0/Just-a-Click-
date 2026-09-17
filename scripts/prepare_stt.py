"""Download pinned STT benchmark assets: whisper.cpp builds, models, FLEURS Korean test."""
import argparse
import json
from pathlib import Path

from prepare_llm import download, download_model, safe_extract_zip

ROOT = Path(__file__).resolve().parents[1]
RANGED_THRESHOLD_BYTES = 64 * 1024 * 1024


def load_config(name):
    return json.loads((ROOT / 'config' / name).read_text('utf-8'))


def runtime_jobs(runtime):
    """(url, archive, size, sha256, extract_dir) for each pinned whisper.cpp build."""
    return [(f"{runtime['release_url']}/{asset['filename']}",
             ROOT / 'downloads/whisper.cpp' / runtime['tag'] / asset['filename'],
             asset['size_bytes'], asset['sha256'],
             ROOT / 'runtimes' / f"whisper-{runtime['tag']}" / asset['build'])
            for asset in runtime['assets']]


def model_jobs(models, selected=None):
    base = f"https://huggingface.co/{models['repository']}/resolve/{models['revision']}"
    return [(f"{base}/{model['filename']}", ROOT / 'models/whisper' / model['filename'],
             model['size_bytes'], model['sha256'])
            for model in models['models'] if not selected or model['id'] in selected]


def data_jobs(data):
    base = f"https://huggingface.co/datasets/{data['dataset']}/resolve/{data['revision']}"
    return [(f"{base}/{item['path']}",
             ROOT / 'downloads/fleurs' / data['config'] / Path(item['path']).name,
             item['size_bytes'], item['sha256'])
            for item in data['files']]


def fetcher_for(size):
    """Large files use resumable byte ranges; small ones a single stream."""
    return download_model if size > RANGED_THRESHOLD_BYTES else download


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--models', nargs='*', help='model ids to fetch (default: all)')
    args = parser.parse_args()
    models = load_config('stt-models.json')
    unknown = set(args.models or []) - {model['id'] for model in models['models']}
    if unknown:
        parser.error(f"Unknown model ids: {', '.join(sorted(unknown))}")
    for url, archive, size, digest, destination in runtime_jobs(load_config('stt-runtime.json')):
        download(url, archive, size, digest)
        safe_extract_zip(archive, destination)
    for url, target, size, digest in data_jobs(load_config('stt-eval-data.json')):
        fetcher_for(size)(url, target, size, digest)
    for url, target, size, digest in model_jobs(models, args.models):
        fetcher_for(size)(url, target, size, digest)


if __name__ == '__main__':
    main()
