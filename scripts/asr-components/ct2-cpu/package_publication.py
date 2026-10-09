"""Construct narrow, deterministic release candidates locally after native proof.

Never publishes, uploads, downloads, assembles a runtime or authorizes release.
Only a separate reviewed request can authorize the exact resulting public bytes.
"""
from __future__ import annotations
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tarfile
import zipfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
WHEEL = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
SOURCES = 'luma-ct2-cpu-4.8.2-1-sources.zip'
NOTICES = 'luma-ct2-cpu-4.8.2-1-notices.zip'
PROOF = 'publication-proof.json'
MAX_BYTES = 100_000_000
CT2_ROOT = 'CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9'
MODEL_PREFIX = CT2_ROOT + '/tests/data/models/'
# These five unused upstream test payloads are the entire omission permission.
# BUILD_TESTS stays OFF, and the original hash-locked source remains the build input.
MODEL_FIXTURES = {
    'v1/aren-transliteration-i16/model.bin': (391462, '0679202c8d70910962443093d3c9b981374bbf176677592e8b2947329b7e230e'),
    'v1/aren-transliteration/model.bin': (744294, 'dcc65eeba74b8cef968489e6b7740af9fea9f42ce45a567f2b95b176e851c39f'),
    'v2/aren-transliteration-i16/model.bin': (392956, '199790bdebb09c4552fef17baa261e3f14661cafb123bd4f75162735f4b78eab'),
    'v2/aren-transliteration-i8/model.bin': (233984, 'f99f62a5b5037a951c36be758668457cf7c13af8ce19cdd15b2f0d45d0f073f2'),
    'v2/aren-transliteration/model.bin': (741720, '8870c0c32eb5dcce3f0a004cd5e13383d365d16d905a6f3c8d9732b98a30112d'),
}
BUILD_STRATEGY = {'kind': 'fresh-fixed-native-root', 'native_root_relative': 'native-build',
                  'builds': 2, 'native_object_cache_reused': False,
                  'path_independence_claim': False, 'cross_machine_claim': False}
SOURCE_TRANSFORMATION = 'Only five reviewed unused model.bin test fixtures omitted; all retained tar records are byte-for-byte unchanged.'
DIST_INFO = 'ctranslate2-4.8.2.dist-info/'
METADATA = {'METADATA', 'WHEEL', 'RECORD', 'entry_points.txt', 'top_level.txt', 'LUMA_CPU_BUILD.json'}
NATIVE = {'ctranslate2/ctranslate2.dll', 'ctranslate2/_ext.cp312-win_amd64.pyd'}
# This explicit source-only closure includes the scripts used by the native proof.
# PBS, build-tool executables and inference inputs are fetched from pinned upstream
# locations when reproducing; their bytes must never enter these public archives.
RECIPE_FILES = (
    'LICENSE', '.github/workflows/asr-ct2-cpu.yml',
    'scripts/prepare-release.cjs',
    'scripts/asr-components/ct2-cpu/proof_request.cjs',
    'scripts/asr-components/ct2-cpu/proof_request.node-test.cjs',
    'scripts/asr-components/build.py', 'scripts/asr-components/assemble.py',
    'scripts/asr-components/packs.json', 'scripts/asr-components/fixtures.json',
    'scripts/asr-components/locks/faster-whisper-cpu-windows-x64.json',
    'src-tauri/src/asr/worker.py', 'src-tauri/src/asr/nagisa_compat.py',
    'scripts/asr-components/ct2-cpu/build_cpu.py',
    'scripts/asr-components/ct2-cpu/package_publication.py',
    'scripts/asr-components/ct2-cpu/prepare_toolchain.ps1',
    'scripts/asr-components/ct2-cpu/cmd_environment.ps1',
    'scripts/asr-components/ct2-cpu/license_inventory.py',
    'scripts/asr-components/ct2-cpu/crt_proof.py',
    'scripts/asr-components/ct2-cpu/native_inventory.py',
    'scripts/asr-components/ct2-cpu/repro_diagnostics.py',
    'scripts/asr-components/ct2-cpu/verify_runtime.py',
    'scripts/asr-components/ct2-cpu/sources.lock.json',
    'scripts/asr-components/ct2-cpu/notices.lock.json',
)


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + '\n').encode('utf-8')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def safe_name(name):
    p = PurePosixPath(name)
    if not isinstance(name, str) or not name or p.is_absolute() or '\\' in name or ':' in name or any(x in ('', '.', '..') for x in name.split('/')):
        raise ValueError('Unsafe public archive path')
    return name


def read_plain(path, root=None):
    path = Path(path)
    if root is not None:
        root = Path(root).resolve()
        if not path.absolute().is_relative_to(root):
            raise ValueError('Public input escaped its root')
        for parent in [path, *path.parents]:
            if parent.is_symlink():
                raise ValueError('Public input includes a symlink')
            if parent == root:
                break
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size < MAX_BYTES:
        raise ValueError('Invalid regular public input')
    return path.read_bytes()


def repository_bytes(source_sha, name):
    """Use immutable Git blob bytes; Windows checkout CRLF is not a new pin."""
    safe_name(name)
    local = read_plain(REPO / name, REPO)
    result = subprocess.run(['git', 'show', source_sha + ':' + name], cwd=REPO,
                            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
    data = result.stdout
    if not 0 < len(data) < MAX_BYTES or local.replace(b'\r\n', b'\n') != data.replace(b'\r\n', b'\n'):
        raise ValueError('Working build recipe differs from the exact source commit')
    return data


def load(path):
    return json.loads(read_plain(path).decode('utf-8-sig'))


def pin(name, data):
    return {'name': name, 'bytes': len(data), 'sha256': sha(data)}


def locked_bytes(item, cache):
    if (not isinstance(item.get('bytes'), int) or isinstance(item['bytes'], bool)
            or not 0 < item['bytes'] < MAX_BYTES or not re.fullmatch('[a-f0-9]{64}', item.get('sha256', ''))):
        raise ValueError('Invalid locked input identity')
    data = read_plain(Path(cache) / item['sha256'], cache)
    if len(data) != item['bytes'] or sha(data) != item['sha256']:
        raise ValueError('Public input differs from its exact source pin')
    return data


def source_members(raw):
    """Bounded inventory of regular files/directories, with preserved metadata."""
    records, spans = {}, {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode='r:') as archive:
        members = archive.getmembers()
        if not 0 < len(members) <= 2000:
            raise ValueError('Source member count exceeded')
        total = 0
        for member in members:
            name = safe_name(member.name.rstrip('/'))
            if (name != CT2_ROOT and not name.startswith(CT2_ROOT + '/')) or name in records:
                raise ValueError('Source root/path is unexpected or duplicated')
            if not member.isfile() and not member.isdir():
                raise ValueError('Source export permits regular files/directories only')
            total += member.size
            if total >= MAX_BYTES or member.size < 0 or (member.isdir() and member.size != 0):
                raise ValueError('Source member byte bounds exceeded')
            data = archive.extractfile(member).read() if member.isfile() else b''
            if len(data) != member.size:
                raise ValueError('Source member is truncated')
            records[name] = {'name': name, 'bytes': member.size, 'sha256': sha(data),
                'type': 'file' if member.isfile() else 'directory', 'mode': member.mode,
                'uid': member.uid, 'gid': member.gid, 'uname': member.uname, 'gname': member.gname,
                'mtime': member.mtime, 'pax_headers': member.pax_headers,
                'linkname': member.linkname, 'devmajor': member.devmajor, 'devminor': member.devminor}
            end = member.offset_data + ((member.size + 511) // 512) * 512
            if not 0 <= member.offset < member.offset_data <= end <= len(raw):
                raise ValueError('Source tar record bounds are invalid')
            spans[name] = (member.offset, end)
    return records, spans


def export_ct2_source(item, original, build_tests):
    """Remove exactly five locked test-model tar records without rewriting others."""
    if (item.get('name') != 'ctranslate2' or item.get('commit') != CT2_ROOT.split('-', 1)[1]
            or build_tests != 'OFF' or len(original) != item.get('bytes') or sha(original) != item.get('sha256')):
        raise ValueError('Exact original CT2 archive and BUILD_TESTS=OFF required')
    with gzip.GzipFile(fileobj=io.BytesIO(original), mode='rb') as stream:
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) >= MAX_BYTES:
        raise ValueError('CT2 source expansion exceeds bounds')
    before, spans = source_members(raw)
    omitted = []
    for suffix, (size, digest) in sorted(MODEL_FIXTURES.items()):
        name = MODEL_PREFIX + suffix
        item_record = before.get(name)
        if (item_record is None or item_record['type'] != 'file'
                or item_record['bytes'] != size or item_record['sha256'] != digest):
            raise ValueError('Reviewed upstream test-model omission set or bytes changed')
        omitted.append({'name': name, 'bytes': size, 'sha256': digest})
    expected_names = {x['name'] for x in omitted}
    actual_models = {name for name in before if name.startswith(MODEL_PREFIX) and name.endswith('/model.bin')}
    if actual_models != expected_names:
        raise ValueError('Unexpected model payload outside the exact reviewed omission set')
    # Preserve the original uncompressed tar bytes except the five complete record
    # spans. No source, license, vocabulary, retained metadata or build input changes.
    pieces, cursor = [], 0
    for start, end in sorted(spans[name] for name in expected_names):
        if start < cursor:
            raise ValueError('Overlapping source omission records')
        pieces.append(raw[cursor:start]); cursor = end
    pieces.append(raw[cursor:])
    exported = b''.join(pieces)
    retained = {name: record for name, record in before.items() if name not in expected_names}
    after, _ = source_members(exported)
    if after != retained:
        raise ValueError('Source export changed a retained member or its metadata')
    original_name = 'upstream/ctranslate2-' + item['commit'] + '.tar.gz'
    export_name = 'upstream/ctranslate2-' + item['commit'] + '-source-only.tar'
    inventory = {'component': 'ctranslate2', 'build_tests': 'OFF', 'transformation': SOURCE_TRANSFORMATION,
                 'original': dict(pin(original_name, original), url=item['url']),
                 'exported': pin(export_name, exported), 'omitted_members': omitted,
                 'retained_member_count': len(retained),
                 'retained_members_sha256': sha(encoded([retained[name] for name in sorted(retained)])),
                 'retained_member_records_preserved': True}
    return exported, inventory


def write_archive(path, entries):
    if not entries or len(entries) > 2000 or sum(len(x) for x in entries.values()) >= MAX_BYTES:
        raise ValueError('Public source archive bounds exceeded')
    with zipfile.ZipFile(path, 'x', compression=zipfile.ZIP_STORED) as archive:
        for name, data in sorted(entries.items()):
            safe_name(name)
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data)


def verify_wheel(wheel, provenance, notices):
    data = read_plain(wheel)
    if Path(wheel).name != WHEEL:
        raise ValueError('Wrong CPU wheel name')
    with zipfile.ZipFile(wheel) as archive:
        entries = archive.infolist()
        if len(entries) > 2000 or len({x.filename for x in entries}) != len(entries) or sum(x.file_size for x in entries) >= MAX_BYTES:
            raise ValueError('Wheel archive is oversized or has duplicate entries')
        binaries = set()
        for item in entries:
            name = safe_name(item.filename)
            if item.is_dir() or (stat.S_IFMT(item.external_attr >> 16) not in (0, stat.S_IFREG)):
                raise ValueError('Wheel may only contain regular entries')
            if item.flag_bits & 1 or not (name.startswith('ctranslate2/') or name.startswith(DIST_INFO)):
                raise ValueError('Unexpected wheel payload scope')
            if name.startswith(DIST_INFO):
                relative = name[len(DIST_INFO):]
                if relative not in METADATA and relative not in notices:
                    raise ValueError('Unexpected wheel metadata or hidden payload')
                if item.file_size > 1_000_000:
                    raise ValueError('Oversized wheel metadata')
            if name.lower().endswith(('.dll', '.pyd', '.exe', '.so', '.dylib')):
                binaries.add(name)
            elif not name.startswith('ctranslate2-4.8.2.dist-info/') and not name.endswith('.py'):
                raise ValueError('Unexpected non-source Python package payload')
        if binaries != NATIVE:
            raise ValueError('Wheel contains an unexpected runtime/CRT/native binary')
        if json.loads(archive.read('ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json')) != provenance:
            raise ValueError('Wheel provenance differs from actual native build')
        for name, expected in notices.items():
            if archive.read('ctranslate2-4.8.2.dist-info/' + name) != expected:
                raise ValueError('Wheel is missing an exact native notice')
    return data


def validate_evidence(source_sha, wheel, second_wheel, reports, lock, work):
    result = load(reports / 'result.json')
    provenance = load(reports / 'provenance.json')
    comparison = load(reports / 'wheel-comparison.json')
    native = load(reports / 'whole-runtime-native.json')
    inference = load(reports / 'inference.json')
    compiler = load(reports / 'compiler-probe.json')
    crt = load(reports / 'private-crt-proof.json')
    if (result.get('source_sha') != source_sha or result.get('publication_authorized') is not False
            or any(result.get(k) is not True for k in ('passed', 'wheel_reproduced', 'native_inference_passed'))):
        raise ValueError('Publication candidates require the successful exact native proof')
    left, right = read_plain(wheel), read_plain(second_wheel)
    if (left != right or result.get('wheel') != dict(filename=WHEEL, bytes=len(left), sha256=sha(left))
            or result.get('second_sha256') != sha(right) or comparison.get('identical') is not True
            or comparison.get('binary_bytes_modified') is not False or comparison.get('different_members') != []
            or comparison.get('first_sha256') != sha(left) or comparison.get('second_sha256') != sha(right)):
        raise ValueError('Missing exact two-build wheel repeatability')
    if (work / 'native-build').exists() or (work / 'native-build').is_symlink():
        raise ValueError('Canonical native build root was not removed after the second build')
    for number, wheel_hash in ((1, sha(left)), (2, sha(right))):
        fresh = load(reports / f'build-{number}-freshness.json')
        if (fresh.get('schema') != 1 or fresh.get('build') != number
                or fresh.get('canonical_native_root') != str(work / 'native-build')
                or any(fresh.get(k) is not True for k in ('fresh_native_root', 'sources_reextracted', 'native_root_removed'))
                or any(fresh.get(k) is not False for k in ('native_object_cache_reused', 'path_independence_claim', 'cross_machine_claim'))
                or fresh.get('retained_wheel_sha256') != wheel_hash):
            raise ValueError('Fresh same-root native build evidence or retained wheel binding differs')
    if (native.get('passed') is not True or native.get('normal_and_delay_imports') is not True
            or native.get('blocked_dependencies') != [] or native.get('forbidden_files') != []
            or not native.get('files')):
        raise ValueError('Private whole-runtime dependency closure is incomplete')
    if (any(inference.get(k) is not True for k in ('passed', 'isolated', 'relocated', 'offline_audit_enabled', 'inherited_python_path_ignored'))
            or inference.get('system_python_used') is not False or inference.get('host_crt_fallback_allowed') is not False
            or inference.get('inference', {}).get('cold_and_warm') is not True
            or inference['inference'].get('cpu_only') is not True or not inference['inference'].get('segments')
            or inference['inference'].get('model') != 'SYSTRAN/faster-whisper-tiny'
            or not {'float32', 'int8'}.issubset(inference.get('compute_types', []))
            or not inference.get('loaded_modules') or compiler.get('passed') is not True):
        raise ValueError('Real isolated Tiny CPU cold/warm inference is incomplete')
    for item in inference['loaded_modules']:
        name = item.get('path', '').replace('\\', '/').rsplit('/', 1)[-1].lower()
        if (item.get('scope') not in {'private', 'windows_os'}
                or re.search(r'cudnn|cublas|cudart|nvrtc|nvcuda|iomp|libomp|vcomp|mkl|tbb', name)
                or (name.startswith(('msvcp', 'vcruntime', 'concrt')) and item['scope'] != 'private')):
            raise ValueError('Actual loaded-module evidence includes an unsafe dependency')
    if (crt.get('public_redistribution_authorized') is not False or crt.get('redistribution_grant_verified') is not False
            or crt.get('original_files_unmodified') is not True or crt.get('global_installation_performed') is not False
            or not crt.get('files')):
        raise ValueError('Private CRT technical proof must retain its redistribution hold')
    if provenance.get('build_strategy') != BUILD_STRATEGY:
        raise ValueError('Build repeatability strategy is not the reviewed fresh fixed-root scope')
    if (provenance.get('schema') != 1 or provenance.get('luma_source_sha') != source_sha
            or provenance.get('variant') != lock['variant'] or provenance.get('publication_authorized') is not False
            or provenance.get('upstream') != lock['sources'] or provenance.get('build_wheels') != lock['build_wheels']
            or provenance.get('ct2_cmake') != lock['ct2_cmake'] or provenance.get('onednn_cmake') != lock['onednn_cmake']):
        raise ValueError('Source/build provenance differs from the locked inputs')
    if (provenance.get('native_compile_flags') != compiler.get('compiler_flags')
            or provenance.get('native_compile_flags') != r'/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\luma-ct2-build'
            or provenance.get('native_link_flags') != '/Brepro /INCREMENTAL:NO'):
        raise ValueError('Compiler flags differ from the exact successful native probe')
    return provenance


def package_publication(*, source_sha, wheel, second_wheel, reports, cache, work, publication_output=None):
    """Create four local candidates and a deterministic report; never publish."""
    if not re.fullmatch('[a-f0-9]{40}', source_sha):
        raise ValueError('Exact source SHA required')
    reports, cache, work = (Path(x).resolve() for x in (reports, cache, work))
    # Import lazily to avoid a build_cpu/package_publication circular import.
    from build_cpu import validate_lock
    lock = load(HERE / 'sources.lock.json'); validate_lock(lock)
    notice_lock = load(HERE / 'notices.lock.json')
    if (notice_lock.get('schema') != 1 or notice_lock.get('notice_inputs_complete') is not True
            or notice_lock.get('blockers') != [] or notice_lock.get('independent_redistribution_review_passed') is not False):
        raise ValueError('Missing reviewed notices or widened redistribution authorization')
    notices = {}
    for item in notice_lock['files']:
        name = safe_name(item['path'])
        if not name.startswith('licenses/') or name in notices:
            raise ValueError('Unexpected or duplicate notice path')
        data = read_plain(HERE / name, HERE)
        if len(data) != item['bytes'] or sha(data) != item['sha256']:
            raise ValueError('Notice bytes differ from the locked review')
        notices[name] = data
    provenance = validate_evidence(source_sha, Path(wheel), Path(second_wheel), reports, lock, work)
    if provenance.get('notices') != notice_lock:
        raise ValueError('Native provenance notice lock differs')
    wheel_bytes = verify_wheel(wheel, provenance, notices)
    source_entries, source_exports = {}, []
    for item in lock['sources']:
        name = safe_name(f'upstream/{item["name"]}-{item["commit"]}.tar.gz')
        if name in source_entries:
            raise ValueError('Duplicate native source')
        data = locked_bytes(item, cache)
        if item['name'] == 'ctranslate2':
            data, inventory = export_ct2_source(item, data, lock['ct2_cmake']['BUILD_TESTS'])
            source_exports.append(inventory); name = inventory['exported']['name']
        source_entries[name] = data
    pybind = [x for x in lock['build_wheels'] if x['name'] == 'pybind11']
    if len(pybind) != 1 or pybind[0]['filename'] != 'pybind11-2.11.1-py3-none-any.whl':
        raise ValueError('Unreviewed header source')
    header_bytes = locked_bytes(pybind[0], cache)
    # This exact pure-Python/header wheel supplies the full pybind11 compilation source.
    source_entries['upstream/' + pybind[0]['filename']] = header_bytes
    for name in RECIPE_FILES:
        source_entries['luma/' + name] = repository_bytes(source_sha, name)
    for name, data in notices.items():
        source_entries['luma/scripts/asr-components/ct2-cpu/' + name] = data
    source_entries['BUILD-PROVENANCE.json'] = encoded(provenance)
    source_entries['SOURCE-EXPORTS.json'] = encoded(source_exports)
    source_entries['README.txt'] = (
        'CPU-only CTranslate2 source bundle. Native sources and pybind11 headers retain their upstream licenses.\n'
        'Luma build/proof scripts are included under the repository LICENSE (GPL-3.0); they are not incorporated into the wheel.\n'
        f'Exact Luma source: {source_sha}\n'
        'The clearly labeled CTranslate2 source-only tar omits exactly five unused upstream model.bin test fixtures because BUILD_TESTS=OFF. All other tar records, source, notices, vocabulary and metadata are unchanged.\n'
        'SOURCE-EXPORTS.json inventories the original archive pin, every omitted member and the deterministic exported archive pin. The original archive remains the exact build input.\n'
        'Use a checkout of that exact commit and the included locked build recipes. Original Python, compiler, build-tool and inference inputs must be obtained directly from their pinned upstream URLs.\n'
        'No Python runtime, Microsoft CRT, FFmpeg, model, or third-party runtime wheel is redistributed here.\n'
    ).encode()
    notice_entries = dict(notices)
    notice_entries['notices.lock.json'] = repository_bytes(source_sha, 'scripts/asr-components/ct2-cpu/notices.lock.json')
    notice_entries['BUILD-PROVENANCE.json'] = encoded(provenance)
    candidate = work / 'publication-candidate'
    if candidate.exists() or candidate.is_symlink():
        raise ValueError('Candidate directory must be fresh')
    if publication_output is not None and (Path(publication_output).exists() or Path(publication_output).is_symlink()):
        raise ValueError('Publication export directory must be fresh')
    candidate.mkdir(parents=True)
    (candidate / WHEEL).write_bytes(wheel_bytes)
    write_archive(candidate / SOURCES, source_entries)
    write_archive(candidate / NOTICES, notice_entries)
    assets = [pin(name, read_plain(candidate / name, candidate)) for name in sorted((WHEEL, SOURCES, NOTICES))]
    locks = {key: pin(name, repository_bytes(source_sha, 'scripts/asr-components/ct2-cpu/' + name)) for key, name in (('sources', 'sources.lock.json'), ('notices', 'notices.lock.json'))}
    proof = {'schema': 1, 'source_sha': source_sha, 'variant': lock['variant'], 'publication_authorized': False,
             'assets': assets, 'locks': locks, 'source_exports': source_exports, 'checks': dict.fromkeys(('wheel_reproduced', 'whole_runtime_closure',
             'native_inference', 'cold_and_warm', 'isolated', 'private_crt', 'cpu_only', 'compiler_probe'), True),
             'provenance': provenance}
    payload = encoded(proof)
    (candidate / PROOF).write_bytes(payload)
    (reports / PROOF).write_bytes(payload)
    if publication_output is not None:
        shutil.copytree(candidate, publication_output)
    return proof
