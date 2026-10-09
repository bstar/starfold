#!/usr/bin/env python3
"""Exercise the real archive provider. Fixtures/output stay outside the checkout.

Use --libarchive with a libarchive checkout to decode selected upstream .uu
fixtures. No fixture downloads or third-party bytes are committed to FOLD.
Additional samples may be supplied with --fixtures FORMAT=PATH (repeatable).
"""
import argparse
import binascii
import bz2
import gzip
import hashlib
import io
import json
import lzma
import pathlib
import resource
import shutil
import struct
import subprocess
import tempfile

FORMATS = '7z AR ARJ BZ2 CAB CHM CPIO DMG EXE FAT GZ ISO LHA LZ4 LZH LZX MSI NTFS PKG QCOW2 RAR RPM SEA SIT SITX SquashFS TAR TAR.BZ2 TAR.GZ TAR.LZ4 TAR.XZ TAR.Z VDI VHD VHDX VMDK WIM XAR XZ Z ZIP ZIPX'.split()
CORPUS = {
    'CAB': 'test_read_format_cab_1.cab.uu',
    'AR': 'test_read_format_ar.ar.uu',
    'CPIO': 'test_read_format_cpio_bin_le.cpio.uu',
    'EXE': 'test_read_format_rar_sfx.exe.uu',
    'LZH': 'test_read_format_lha_header0.lzh.uu',
    'LHA': 'test_read_format_lha_amiga_script.lha.uu',
    'RAR': 'test_read_format_rar.rar.uu',
    'RPM': 'test_read_format_cpio_svr4_gzip_rpm.rpm.uu',
    'ZIPX': 'test_read_format_zip_bzip2.zipx.uu',
    'TAR.LZ4': 'test_compat_lz4_1.tar.lz4.uu',
    'TAR.Z': 'test_compat_mac-1.tar.Z.uu',
    'Z': 'test_archive_string_conversion.txt.Z.uu',
}

def request(binary, message):
    frame = json.dumps(message).encode()
    def fixture_limits():
        # This runner tests small fixtures, never arbitrarily large expansion.
        resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024 * 1024, 64 * 1024 * 1024))
    with tempfile.TemporaryFile() as spool:
        completed = subprocess.run([str(binary), '--stdio'], input=struct.pack('<I', len(frame)) + frame,
                                   stdout=spool, stderr=subprocess.PIPE, timeout=60, preexec_fn=fixture_limits)
        if completed.returncode:
            raise RuntimeError('provider exited: ' + completed.stderr.decode(errors='replace')[-1000:])
        spool.seek(0)
        raw = spool.read(64 * 1024 * 1024 + 1)
    if len(raw) > 64 * 1024 * 1024:
        raise RuntimeError('fixture output exceeds matrix budget')
    output = io.BytesIO(raw)
    entries, data = [], bytearray()
    done = False
    while output.tell() < len(raw):
        size = output.read(4)
        if len(size) != 4:
            raise RuntimeError('truncated protocol frame')
        size = struct.unpack('<I', size)[0]
        if size > 32 * 1024 * 1024:
            raise RuntimeError('oversized protocol frame')
        reply = json.loads(output.read(size))
        if reply == 'Done':
            done = True
        elif 'Error' in reply:
            raise RuntimeError(reply['Error']['message'])
        elif 'Entries' in reply:
            if reply['Entries']['partial']:
                raise RuntimeError('partial index')
            entries = reply['Entries']['entries']
        elif 'Data' in reply:
            length = reply['Data']['length']
            if length > 256 * 1024:
                raise RuntimeError('oversized member chunk')
            chunk = output.read(length)
            if len(chunk) != length:
                raise RuntimeError('truncated member data')
            data.extend(chunk)
    if not done:
        raise RuntimeError('provider did not complete request')
    return entries, bytes(data)


def fixtures(binary, root):
    source = root / 'hello.txt'
    source.write_bytes(b'archive matrix fixture\n')
    generated = {}
    options = dict(preset='Balanced', password=None, volume_bytes=None, exclude_mac_metadata=False)
    for label, variant, suffix in [('ZIP', 'Zip', 'zip'), ('7z', 'SevenZip', '7z'), ('TAR', 'Tar', 'tar'),
                                   ('TAR.GZ', 'TarGz', 'tar.gz'), ('TAR.ZST', 'TarZst', 'tar.zst')]:
        output = root / ('sample.' + suffix)
        request(binary, {'Create': dict(format=variant, output=str(output), options=options,
                                       items=[{'from': str(source), 'to': 'hello.txt', 'kind': {'File': source.stat().st_size}}])})
        generated[label] = output
    for label, suffix, compressor in [('TAR.BZ2', 'tar.bz2', bz2.compress), ('TAR.XZ', 'tar.xz', lzma.compress)]:
        output = root / ('sample.' + suffix)
        output.write_bytes(compressor(generated['TAR'].read_bytes()))
        generated[label] = output
    for label, suffix, compressor in [('GZ', 'gz', gzip.compress), ('BZ2', 'bz2', bz2.compress), ('XZ', 'xz', lzma.compress)]:
        output = root / ('hello.' + suffix)
        output.write_bytes(compressor(source.read_bytes()))
        generated[label] = output
    if shutil.which('bsdtar'):
        for label, format_name, suffix in [('ISO', 'iso9660', 'iso'), ('XAR', 'xar', 'xar')]:
            output = root / ('sample.' + suffix)
            result = subprocess.run(['bsdtar', '--format', format_name, '-cf', str(output), '-C', str(root), 'hello.txt'],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            if result.returncode == 0:
                generated[label] = output
    if shutil.which('ar'):
        output = root / 'sample.ar'
        subprocess.run(['ar', 'qc', str(output), str(source)], check=True, stdout=subprocess.DEVNULL)
        generated['AR'] = output
    return generated


def decode_uu(path, output):
    lines = path.read_bytes().splitlines()
    active = False
    with output.open('wb') as file:
        for line in lines:
            if line.startswith(b'begin '):
                active = True
                continue
            if not active:
                continue
            if line == b'end':
                break
            if line:
                file.write(binascii.a2b_uu(line))
            if file.tell() > 16 * 1024 * 1024:
                raise RuntimeError('corpus sample exceeds fixture limit')


def run_case(binary, label, path, output, provenance):
    row = dict(format=label, fixture=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
               provenance=provenance, browse=False, read=False, extract=False, test=False, renamed=False)
    checks = [('browse', {'List': dict(source=str(path), limit=100000, password=None)}),
              ('extract', {'Extract': dict(source=str(path), output=str(output), password=None)}),
              ('test', {'Test': dict(source=str(path), password=None)})]
    errors = {}
    entries = []
    for name, message in checks:
        try:
            result, _ = request(binary, message)
            row[name] = True
            if name == 'browse':
                entries = result
        except Exception as error:
            errors[name] = str(error)
    regular = next(((i, e) for i, e in enumerate(entries)
                    if not e['directory'] and (e.get('bytes') is None or e['bytes'] <= 1024 * 1024)), None)
    if regular:
        i, entry = regular
        try:
            _, data = request(binary, {'Read': dict(source=str(path), index=i, password=None)})
            extracted = output / entry['name']
            row['read'] = (entry.get('bytes') is None or len(data) == entry['bytes']) and (not row['extract'] or extracted.read_bytes() == data)
            if not row['read']:
                errors['read'] = 'member size or extracted contents differ'
        except Exception as error:
            errors['read'] = str(error)
    else:
        errors['read'] = 'fixture has no bounded regular member'
    renamed = path.with_name(path.name + '.renamed-data')
    shutil.copyfile(path, renamed)
    try:
        request(binary, {'Inspect': dict(source=str(renamed))})
        row['renamed'] = True
    except Exception as error:
        errors['renamed'] = str(error)
    row['errors'] = errors
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--libarchive', type=pathlib.Path)
    parser.add_argument('--fixtures', action='append', default=[], metavar='FORMAT=PATH')
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='starfold-format-matrix-') as folder:
        root = pathlib.Path(folder)
        samples = fixtures(binary, root)
        provenance = {label: 'generated from current FOLD archive extension' for label in samples}
        if args.libarchive:
            revision = subprocess.check_output(['git', '-C', str(args.libarchive), 'rev-parse', 'HEAD'], text=True).strip()
            for label, name in CORPUS.items():
                source = args.libarchive / 'libarchive/test' / name
                if source.exists():
                    sample = root / name.removesuffix('.uu')
                    decode_uu(source, sample)
                    samples[label] = sample
                    provenance[label] = f'https://github.com/libarchive/libarchive/blob/{revision}/libarchive/test/{name}'
        for supplied in args.fixtures:
            label, path = supplied.split('=', 1)
            samples[label] = pathlib.Path(path).resolve()
            provenance[label] = 'user-supplied fixture'
        rows = []
        for label in FORMATS + ['TAR.ZST']:
            if label in samples:
                row = run_case(binary, label, samples[label], root / ('extract-' + label), provenance[label])
                print(label, 'PASS' if all(row[k] for k in ['browse', 'read', 'extract', 'test', 'renamed']) else 'FAIL', flush=True)
                rows.append(row)
            else:
                rows.append(dict(format=label, status='fixture required'))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(dict(platform=__import__('platform').platform(), rows=rows), indent=2) + '\n')
        return 1 if any('errors' in row and row['errors'] for row in rows) else 0

if __name__ == '__main__':
    raise SystemExit(main())
