#!/usr/bin/env python3
"""Bounded read-only discovery of existing vendor documents in the exact VS tree.

No downloads, guessed license path, agreement acceptance or credentials. A
candidate or end-user license is evidence, never an automatic redistribution grant.
"""
from __future__ import annotations
import hashlib
import html
import os
from pathlib import Path
import re
import zipfile
import xml.etree.ElementTree as ET

DOCUMENT = re.compile(r'license|licence|eula|terms|notice|redist', re.I)
SENSITIVE = re.compile(r'activation|credential|license.?key|product.?key|password|secret|token.?store', re.I)
EXTENSIONS = {'.txt', '.md', '.rst', '.rtf', '.htm', '.html', '.docx', '.pdf'}


def text_from_document(path, limit=256_000):
    if path.suffix.lower() == '.pdf':
        return None, 'PDF candidate: metadata only'
    if path.stat().st_size > limit:
        return None, 'Document exceeds readable-text bound'
    if path.suffix.lower() == '.docx':
        with zipfile.ZipFile(path) as archive:
            if len(archive.infolist()) > 1000:
                return None, 'DOCX entry bound exceeded'
            entry = archive.getinfo('word/document.xml')
            if entry.file_size > limit:
                return None, 'DOCX text exceeds bound'
            data = archive.read(entry)
            if b'<!DOCTYPE' in data.upper() or b'<!ENTITY' in data.upper():
                return None, 'External/XML entity declarations are not allowed'
            root = ET.fromstring(data)
            return '\n'.join(''.join(p.itertext()) for p in root.iter('{http://schemas.openxmlformats.org/wordprocessingml/2006/main}p')), None
    data = path.read_bytes()
    text = data.decode('utf-16' if data.startswith((b'\xff\xfe', b'\xfe\xff')) else 'utf-8-sig', errors='replace')
    if path.suffix.lower() in {'.html', '.htm'}:
        text = re.sub(r'(?is)<(script|style)\b.*?</\1\s*>', '', text)
        text = html.unescape(re.sub(r'<[^>]+>', ' ', text))
    return text, None


def discover(installation, redist, product_id, version, *, max_files=100_000, max_documents=200, max_text_total=1_000_000):
    installation, redist = Path(installation).resolve(), Path(redist).resolve()
    if not installation.is_dir() or not redist.is_dir() or not redist.is_relative_to(installation):
        raise ValueError('License discovery requires the exact VS installation and its contained Redist directory')
    report = {'schema': 1, 'product_id': product_id, 'product_version': version,
              'installation_root': str(installation), 'redist_root': str(redist),
              'documents': [], 'bounds': {'files': max_files, 'documents': max_documents,
                                         'single_document_bytes': 4_000_000, 'text_total_characters': max_text_total},
              'files_examined': 0, 'directories_examined': 0, 'skipped_reparse_points': 0,
              'skipped_sensitive_names': 0, 'read_errors': [], 'truncated': False,
              'downloads': False, 'agreements_accepted': False, 'redistribution_grant_verified': False}
    seen = set(); seen_dirs = set(); text_total = 0
    # Give the actual native Redist tree priority before the larger VS tree.
    queue = [redist, installation]
    while queue:
        directory = queue.pop(0)
        key = str(directory).casefold()
        if key in seen_dirs:
            continue
        seen_dirs.add(key); report['directories_examined'] += 1
        if report['directories_examined'] > 15_000:
            report['truncated'] = True; break
        try:
            entries = sorted(os.scandir(directory), key=lambda e: e.name.casefold())
        except OSError as error:
            report['read_errors'].append({'path': str(directory), 'error': type(error).__name__}); continue
        for entry in entries:
            path = Path(entry.path)
            if entry.is_symlink() or getattr(path, 'is_junction', lambda: False)():
                report['skipped_reparse_points'] += 1; continue
            if SENSITIVE.search(entry.name):
                report['skipped_sensitive_names'] += 1; continue
            if entry.is_dir(follow_symlinks=False):
                queue.append(path); continue
            if not entry.is_file(follow_symlinks=False):
                continue
            report['files_examined'] += 1
            if report['files_examined'] > max_files or len(report['documents']) >= max_documents:
                report['truncated'] = True; queue.clear(); break
            relative = path.relative_to(installation).as_posix()
            if path.suffix.lower() not in EXTENSIONS or not DOCUMENT.search(relative):
                continue
            if str(path).casefold() in seen:
                continue
            seen.add(str(path).casefold())
            size = path.stat().st_size
            item = {'path': str(path), 'relative_path': relative, 'bytes': size,
                    'under_redist': path.is_relative_to(redist), 'format': path.suffix.lower(), 'terms_recovered': False}
            report['documents'].append(item)
            if size > 4_000_000:
                item['skipped_reason'] = 'Candidate exceeds hash/read bound'; continue
            try:
                with path.open('rb') as stream:
                    item['sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
                text, reason = text_from_document(path)
                if reason:
                    item['text_omitted_reason'] = reason; continue
                normalized = re.sub(r'\s+', ' ', text).strip()
                # Only recognizable Microsoft terms are copied into reports.
                # A pointer/link, product-key label or generic metadata is not terms.
                credential_label = re.search(r'(?i)(?:product.?key|license.?key|activation.?code|password|secret|token)\s*[:=]|\b[A-Z0-9]{5}(?:-[A-Z0-9]{5}){4}\b', normalized)
                recognizable = bool(re.search(r'(?i)Microsoft.{0,80}(?:software license terms|visual.{0,20}c\+\+|visual studio)', normalized)
                                    and re.search(r'(?i)(?:redistribut|distributable code|license terms|licensed.{0,30}software)', normalized))
                link_only = bool(re.fullmatch(r'https?://\S+', normalized))
                item['link_only'] = link_only
                if not recognizable or link_only or credential_label:
                    item['text_omitted_reason'] = 'Not recognizable vendor terms, link-only, or possible credential label'; continue
                if text_total + len(text) > max_text_total:
                    item['text_omitted_reason'] = 'Aggregate text bound exceeded'; continue
                item['text'] = text; text_total += len(text)
                item['terms_recovered'] = True
                item['interpretation'] = 'Existing document text only; scope and redistribution grant require independent review.'
            except (OSError, ValueError, KeyError, zipfile.BadZipFile, ET.ParseError) as error:
                item['read_error'] = type(error).__name__
    return report
