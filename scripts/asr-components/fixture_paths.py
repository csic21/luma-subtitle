"""Test-only ownership boundary for canonical temporary fixture roots.

Use only when creating a fixture, before deriving child paths. This is not an
input sanitizer: never apply it to paths that a test intentionally makes unsafe,
noncanonical, symlinked or outside a production guard's trusted root.
"""
from contextlib import contextmanager
from pathlib import Path
import os
import tempfile


@contextmanager
def temporary_root(*, prefix='luma-test-', suffix='', dir=None):
    """Own a new temporary directory and yield its canonical pathlib.Path."""
    with tempfile.TemporaryDirectory(prefix=prefix, suffix=suffix, dir=dir) as name:
        root = Path(name).resolve(strict=True)
        if not root.is_dir():
            raise ValueError('Owned temporary fixture root is not a directory')
        yield root


def windows_short_path_alias(owned_path):
    """Return an actual distinct 8.3 alias, or None when unavailable/disabled.

    This optional native test probe never creates aliases, changes filesystem
    settings, or normalizes production input. The caller must own the directory.
    Native API errors are surfaced for the test to report or explicitly skip.
    """
    if os.name != 'nt':
        return None
    import ctypes
    from ctypes import wintypes
    canonical = Path(owned_path).resolve(strict=True)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    get_short = kernel.GetShortPathNameW
    get_short.argtypes = (wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD)
    get_short.restype = wintypes.DWORD
    size = get_short(str(canonical), None, 0)
    if not size:
        raise ctypes.WinError(ctypes.get_last_error())
    buffer = ctypes.create_unicode_buffer(size)
    written = get_short(str(canonical), buffer, size)
    if not written or written >= size:
        raise ctypes.WinError(ctypes.get_last_error())
    alias = Path(buffer.value)
    if os.path.normcase(str(alias)) == os.path.normcase(str(canonical)):
        return None
    if alias.resolve(strict=True) != canonical:
        raise ValueError('Native short-path alias resolved to another fixture directory')
    return alias
