"""Use exact original Microsoft CRT files for licensed hosted-CI proof only.

This does not package or authorize redistribution of a runtime or CRT sidecar.
Signatures/version are observed by prepare_toolchain.ps1 before this function;
source bytes are checked again against reviewed hashes before any private copy.
"""
import hashlib
import json
import os
from pathlib import Path

VERSION = '14.44.35211.0'
SIGNER = 'CN=Microsoft Windows Software Compatibility Publisher, O=Microsoft Corporation, L=Redmond, S=Washington, C=US'
EXISTING = {
    'vcruntime140.dll': (124544, 'd5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066', 'EC5F0D7EE2327688384B4FDF5D7633553A0D055F'),
    'vcruntime140_1.dll': (49792, '1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f', 'EC5F0D7EE2327688384B4FDF5D7633553A0D055F'),
}
MISSING = {
    'msvcp140.dll': (557728, '0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9', '81915C173D7FFCBF49EAA8CF7594696B29A035E1'),
    'msvcp140_1.dll': (35952, 'bfad5aef4c63a669e3c140655cdfdf395b6c979b400a447bd5dcb65ed8826c3d', '81915C173D7FFCBF49EAA8CF7594696B29A035E1'),
}
POINTER_SHA256 = 'da53b097e02b08e0fc69706102a60bc384fe756426ae4dc4a855e96f95cb2b9c'


def _load(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def copy_proof_crt(runtime, reports):
    runtime, reports = Path(runtime).resolve(), Path(reports).resolve()
    toolchain = _load(reports / 'toolchain.json')
    report = _load(reports / 'crt-candidates.json')
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or toolchain.get('github_actions') is not True
            or toolchain.get('runner_environment') != 'github-hosted'
            or toolchain.get('product_id') != 'Microsoft.VisualStudio.Product.Enterprise'
            or toolchain.get('visual_studio_version') != '17.14.37710.0'
            or toolchain.get('vc_tools_version') != '14.44.35207'
            or report.get('binary_publication_authorized') is not False):
        raise ValueError('Original CRT copies are restricted to the reviewed hosted-CI proof')
    installation = Path(toolchain['installation_path']).resolve()
    redist = Path(toolchain['redist_path']).resolve()
    if not redist.is_relative_to(installation) or Path(report['official_redist_directory']).resolve() != redist:
        raise ValueError('CRT source is outside the exact discovered Visual Studio Redist tree')
    candidates = report['candidates']
    verified, payloads = [], {}
    for name, (size, sha256, thumbprint) in {**EXISTING, **MISSING}.items():
        matches = [item for item in candidates if item['filename'] == name]
        if len(matches) != 1:
            raise ValueError('Missing or duplicate original CRT identity: ' + name)
        item = matches[0]
        source = Path(item['source_path'])
        expected = redist / 'x64/Microsoft.VC143.CRT' / name
        if (source.is_symlink() or source.resolve() != expected.resolve()
                or not source.resolve().is_relative_to(redist)
                or item['bytes'] != size or item['sha256'] != sha256
                or item['file_version'] != VERSION or item['product_version'] != VERSION
                or item['signature_status'] != 'Valid' or item['signer_subject'] != SIGNER
                or item['signer_thumbprint'] != thumbprint):
            raise ValueError('Original CRT provenance/signature differs from reviewed inventory: ' + name)
        data = source.read_bytes()
        if len(data) != size or hashlib.sha256(data).hexdigest() != sha256:
            raise ValueError('Original CRT source bytes changed: ' + name)
        destination = runtime / name
        if name in EXISTING:
            if destination.is_symlink() or not destination.is_file() or destination.read_bytes() != data:
                raise ValueError('PBS CRT does not match the official same-version source: ' + name)
        else:
            if destination.exists() or destination.is_symlink():
                raise ValueError('Refusing to replace an existing private CRT file: ' + name)
            payloads[name] = data
        verified.append(dict(item, private_filename=name, already_supplied_by_pbs=name in EXISTING))
    licenses = _load(reports / 'installed-license-evidence.json')
    pointers = [item for item in licenses['documents'] if item.get('sha256') == POINTER_SHA256
                and item.get('relative_path') == 'Licenses/1033/Redist.txt']
    if len(pointers) != 1 or pointers[0].get('link_only') is not True or pointers[0].get('terms_recovered') is not False:
        raise ValueError('Observed Redist pointer evidence is missing or incorrectly classified')
    pointer = pointers[0]['pointer_text'].encode('utf-8')
    if len(pointer) != 187 or hashlib.sha256(pointer).hexdigest() != POINTER_SHA256:
        raise ValueError('Observed Redist pointer bytes changed')
    evidence = {'schema': 1, 'purpose': 'Private technical proof in licensed hosted CI only',
                'public_redistribution_authorized': False, 'redistribution_grant_verified': False,
                'original_files_unmodified': True, 'global_installation_performed': False,
                'files': verified, 'copied_names': sorted(payloads),
                'notices': {'existing_pbs_notices_preserved': True,
                            'redist_pointer_sha256': POINTER_SHA256, 'pointer_is_license_grant': False},
                'official_sources': report['official_sources']}
    # Validate every input before writing. Exclusively create the two missing files;
    # never overwrite PBS VCRUNTIME files or substitute renamed package DLLs.
    for name, data in payloads.items():
        with (runtime / name).open('xb') as target:
            target.write(data)
    notices = runtime / 'licenses'; notices.mkdir(exist_ok=True)
    with (notices / 'MICROSOFT-CRT-REDIST-POINTER.txt').open('xb') as target:
        target.write(pointer)
    serialized = json.dumps(evidence, indent=2, sort_keys=True) + '\n'
    (runtime / 'crt-proof-provenance.json').write_text(serialized, encoding='utf-8', newline='\n')
    (reports / 'private-crt-proof.json').write_text(serialized, encoding='utf-8', newline='\n')
    return evidence
