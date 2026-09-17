"""Download pinned M0 assets. Standard library only; not the product installer."""
import hashlib
import json
from pathlib import Path
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor
import shutil

ROOT = Path(__file__).resolve().parents[1]


def verify(path, size, digest):
    if not path.exists() or path.stat().st_size != size:
        return False
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest() == digest


def download(url, target, size=None, digest=None):
    target.parent.mkdir(parents=True, exist_ok=True)
    if size is not None and verify(target, size, digest):
        print(f'Verified existing {target.name}', flush=True)
        return
    temporary = target.with_suffix(target.suffix + '.part')
    print(f'Downloading {target.name}', flush=True)
    request = urllib.request.Request(url, headers={'User-Agent': 'Just-a-Click-M0'})
    with urllib.request.urlopen(request, timeout=60) as response, temporary.open('wb') as output:
        total = 0
        last = 0
        while chunk := response.read(4 * 1024 * 1024):
            output.write(chunk)
            total += len(chunk)
            if total - last >= 256 * 1024 * 1024:
                print(f'{target.name}: {total / 1e9:.2f} GB', flush=True)
                last = total
    if size is not None and not verify(temporary, size, digest):
        raise RuntimeError(f'Size/hash mismatch: {target.name}')
    temporary.replace(target)
    print(f'Installed {target.name}', flush=True)


def download_model(url, target, size, digest):
    """Bounded parallel ranges with resumable chunks and final SHA-256 validation."""
    if verify(target, size, digest):
        print(f'Verified existing {target.name}', flush=True)
        return
    parts = target.parent / (target.name + '.parts')
    parts.mkdir(parents=True, exist_ok=True)
    chunk_size = 64 * 1024 * 1024

    def fetch(index):
        start = index * chunk_size
        end = min(start + chunk_size, size) - 1
        part = parts / str(index)
        have = part.stat().st_size if part.exists() else 0
        if have > end - start + 1:
            part.unlink()
            have = 0
        if have == end - start + 1:
            return part
        # Resume after the bytes already cached; distinct cache keys per offset prevent
        # intermediary caches serializing byte ranges.
        first = start + have
        range_url = url + ('&' if '?' in url else '?') + f'download=true&part={index}&from={have}'
        request = urllib.request.Request(range_url, headers={
            'User-Agent': 'Just-a-Click-M0', 'Range': f'bytes={first}-{end}'})
        with urllib.request.urlopen(request, timeout=90) as response:
            expected = f'bytes {first}-{end}/{size}'
            if response.status != 206 or response.headers.get('Content-Range') != expected:
                raise RuntimeError('Server did not honor exact byte range')
            with part.open('ab') as output:
                shutil.copyfileobj(response, output, 1024 * 1024)
        if part.stat().st_size != end - start + 1:
            raise RuntimeError('Incomplete range')
        print(f'Model chunk {index + 1} complete', flush=True)
        return part

    count = (size + chunk_size - 1) // chunk_size
    with ThreadPoolExecutor(max_workers=8) as executor:
        downloaded = list(executor.map(fetch, range(count)))
    temporary = target.with_suffix(target.suffix + '.part')
    with temporary.open('wb') as output:
        for part in downloaded:
            with part.open('rb') as source:
                shutil.copyfileobj(source, output, 4 * 1024 * 1024)
    if not verify(temporary, size, digest):
        raise RuntimeError('Model SHA-256 mismatch; remove cached chunks before retry')
    temporary.replace(target)
    for part in downloaded:
        part.unlink()
    parts.rmdir()
    print(f'Installed and SHA-256 verified {target.name}', flush=True)


def safe_extract_zip(archive, destination):
    """Extract only when every member stays inside the destination directory."""
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        for member in zipped.infolist():
            if not (destination / member.filename).resolve().is_relative_to(destination.resolve()):
                raise RuntimeError('Unsafe archive path')
        zipped.extractall(destination)


def main():
    model = json.loads((ROOT / 'config/llm-model.json').read_text('utf-8'))['artifact']
    runtime = json.loads((ROOT / 'config/llama-runtime.json').read_text('utf-8'))
    for asset in runtime['assets']:
        archive = ROOT / 'downloads' / asset['filename']
        url = f"https://github.com/ggml-org/llama.cpp/releases/download/{runtime['tag']}/{asset['filename']}"
        download(url, archive, asset['size_bytes'], asset['sha256'])
        safe_extract_zip(archive, ROOT / 'runtimes' / runtime['tag'] / asset['backend'])
    download_model(model['url'], ROOT / 'models' / model['filename'], model['size_bytes'], model['sha256'])
    download(model['license_url'], ROOT / 'models' / 'LICENSE-Qwen3-8B')


if __name__ == '__main__':
    main()
