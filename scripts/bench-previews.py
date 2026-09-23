#!/usr/bin/env python3
"""Reproducible generated-fixture preview benchmark; does not install tools.
Run after a release build: python3 scripts/bench-previews.py target/release/starfold
Numbers are fresh-process or retained-session timings, with warm filesystem cache.
"""
import json
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time
import shutil

CFG = dict(timeout_ms=2000, cache_bytes=33554432, pdf_page_bytes=262144,
           max_bytes=262144, max_lines=400, max_image_dimension=4096, dir_budget=20000)

def make_pdf(path, count):
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'',
               b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>']
    kids = []
    for number in range(1, count + 1):
        page_id = len(objects) + 1
        kids.append(f'{page_id} 0 R')
        text = f'BT /F1 12 Tf 50 700 Td (Page {number} benchmark text) Tj ET'.encode()
        objects.extend([
            f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {page_id+1} 0 R >>'.encode(),
            f'<< /Length {len(text)} >>\nstream\n'.encode() + text + b'\nendstream'])
    objects[1] = f'<< /Type /Pages /Kids [{" ".join(kids)}] /Count {count} >>'.encode()
    data = bytearray(b'%PDF-1.5\n')
    offsets = [0]
    for i, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f'{i} 0 obj\n'.encode() + obj + b'\nendobj\n')
    xref = len(data)
    data.extend(f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:
        data.extend(f'{offset:010d} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    path.write_bytes(data)

def request(child, path, page):
    child.stdin.write(json.dumps(dict(path=list(bytes(path)), page=page, cfg=CFG)) + '\n')
    child.stdin.flush()
    result = json.loads(child.stdout.readline())
    if 'Err' in result:
        raise RuntimeError(result['Err'])
    return result['Ok']

def bench(binary, path):
    initial, more = [], []
    for _ in range(7):
        start = time.perf_counter()
        child = subprocess.Popen([binary, '--preview-worker'], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            result = request(child, path, 1)
            initial.append((time.perf_counter() - start) * 1000)
            if path.suffix == '.pdf':
                assert 'Page 1 benchmark text' in result['content']['Pages'][0]['text']
                start = time.perf_counter()
                request(child, path, 4)
                more.append((time.perf_counter() - start) * 1000)
            elif path.suffix in ('.mp3', '.mp4'):
                assert any(f['label'] == 'Title' and f['value'] == 'Preview benchmark' for f in result['fields']), result
        finally:
            child.stdin.close()
            child.wait(timeout=5)
    print(f'{path.name}: first preview {statistics.median(initial):.2f} ms'
          + (f'; next 3 pages {statistics.median(more):.2f} ms' if more else ''))
    if path.suffix == '.pdf' and shutil.which('pdftotext'):
        args = ['pdftotext', '-f', '1', '-l', '3', str(path), '-']
    elif path.suffix in ('.mp3', '.mp4') and shutil.which('ffprobe'):
        args = ['ffprobe', '-v', 'error', '-show_format', '-show_streams', '-of', 'json', str(path)]
    else:
        return
    reference = []
    for _ in range(7):
        start = time.perf_counter()
        subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
        reference.append((time.perf_counter() - start) * 1000)
    print(f'  {args[0]} reference: {statistics.median(reference):.2f} ms')

if __name__ == '__main__':
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix='starfold-preview-bench-') as tmp:
        root = pathlib.Path(tmp)
        for pages in (10, 200, 1000):
            path = root / f'{pages}-pages.pdf'
            make_pdf(path, pages)
            bench(binary, path)
        if shutil.which('ffmpeg'):
            for ext, source, codec in [('mp3', 'sine=frequency=440:duration=1', 'libmp3lame'),
                                        ('mp4', 'color=c=black:s=160x90:d=1', 'mpeg4')]:
                path = root / f'tagged.{ext}'
                subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', source,
                                '-c:a' if ext == 'mp3' else '-c:v', codec,
                                '-metadata', 'title=Preview benchmark', str(path)], check=True)
                bench(binary, path)
