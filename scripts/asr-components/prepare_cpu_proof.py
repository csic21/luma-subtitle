#!/usr/bin/env python3
"""Prepare only the CPU publication metadata anchored by the shipping lock.

Use the existing bounded hash-cache downloader, then the existing full release
proof validator. A local proof can be supplied for fully offline verification.
This never discovers releases, resolves wheels, builds CT2 or enables a recipe.
"""
import argparse
import json
from pathlib import Path

from build import fetch
from own_cpu_recipe import (MAX_PROOF, PACK_ID, ROOT, checked_bytes, own_wheel,
                            plain_path, publication_pin, published_component)


def shipping_cpu_lock():
    lock = json.loads((ROOT / 'locks' / (PACK_ID + '.json')).read_text(encoding='utf-8'))
    if 'cpu_component' not in lock and not any(own_wheel(wheel) for wheel in lock['wheels']):
        return None  # Old upstream locks require no own-component metadata.
    publication_pin(lock)  # Reject partial/malformed own-component transitions.
    return lock


def prepare(cache, cpu_publication_proof=None):
    lock = shipping_cpu_lock()
    if lock is None:
        return None
    pin = publication_pin(lock)
    raw = None
    if cpu_publication_proof is not None:
        # Verify before creating or changing the cache; never fall back to a
        # download when the caller explicitly supplied invalid offline bytes.
        published_component(lock, cpu_publication_proof, root=ROOT)
        raw = checked_bytes(cpu_publication_proof, pin, MAX_PROOF)
    cache = Path(cache).absolute()
    cache.mkdir(parents=True, exist_ok=True)
    cache = plain_path(cache, directory=True)
    target = cache / pin['sha256']
    partial = target.with_suffix('.partial')
    for path in (target, partial):
        if path.exists() or path.is_symlink():
            plain_path(path)  # Never follow a cache link/reparse point.
    if raw is not None:
        if target.exists():
            checked_bytes(target, pin, MAX_PROOF)
        else:
            # Exclusive local staging cannot overwrite an in-flight download.
            owns_partial = False
            try:
                with partial.open('xb') as output:
                    owns_partial = True
                    output.write(raw)
                checked_bytes(partial, pin, MAX_PROOF)
                partial.replace(target)
            finally:
                if owns_partial:
                    partial.unlink(missing_ok=True)
    else:
        target = fetch(pin, cache)
    published_component(lock, target, root=ROOT)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--cpu-publication-proof', type=Path,
                        help='Exact local lock-pinned proof; this path never downloads.')
    args = parser.parse_args()
    proof = prepare(args.cache, args.cpu_publication_proof)
    if proof is not None:
        print(proof)


if __name__ == '__main__':
    main()
