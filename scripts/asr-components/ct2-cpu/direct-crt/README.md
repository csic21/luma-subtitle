# Direct Microsoft CRT package audit

This directory contains a **report-only proof**, not app integration or a grant
of redistribution rights. The downloaded installer and its DLLs are never run.
There are no global installation, registry, PATH, administrator, or license
acceptance steps. Commit and upload only code and JSON metadata, never the
Microsoft installer, cabinet, DLL, or runtime bytes.

## Exact input and extraction

`package.lock.json` is the compact, data-only identity/mapping for a future
reviewed installer. Its duplicated constants are checked against the audit
helper by unit tests. Changing a package requires a fresh audit; this is not a
generic Burn or MSI extractor.

The sole input is Microsoft's original `VC_redist.x64.exe` version
14.44.35211.0, 25,635,768 bytes, with SHA-256
`cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b`.
The full hash is checked before binary parsing. The verified input contains:

- A UX CAB at offset 479,744, length 195,988.
- An attached CAB at offset 686,152, length 24,939,223.
- Attached member `a12`, a 987,836-byte x64 minimum-runtime CAB.
- Inside `a12`, the two exact members `msvcp140.dll_amd64` and
  `msvcp140_1.dll_amd64`. No MSI database access is needed.
- UX members `u4` and `u28`, the original English and Simplified Chinese runtime
  license RTF documents. Their original bytes are preserved privately. The
  English EULA identifies `Cpp_2015-2022_ENU.1033`.

All intermediate and final lengths/hashes are in the lock. The helper gets the
Windows system directory from `GetSystemDirectoryW` and invokes only its existing
`expand.exe`, by absolute path and fixed exact-member arguments, without a shell.
Each extraction uses a newly created directory and requires exactly the expected
member and pinned bytes. No DLL is loaded and no downloaded executable is run.

## Run

Use the existing orchestration Python 3.12 on Windows, with an absolute new
private work directory and a new report filename whose parent exists:

```powershell
python -B scripts/asr-components/ct2-cpu/direct-crt/audit_direct_crt.py `
  --work-dir 'D:\private-proof\direct-crt-new' `
  --report 'D:\reports\direct-crt-inputs.json'
```

The input manifest has `schema: 1`, `source_url`, `work_root`, and exactly three
`files` entries with `role`, absolute private `path`, `bytes`, and `sha256`.
The roles are `installer`, `msvcp140.dll`, and `msvcp140_1.dll`. `notices` contains
the two privately extracted RTF identities. `authenticode_verified` stays false
here: a separate native PowerShell verifier must independently require Valid
Microsoft Authenticode signatures and report signer/timestamp certificates and
actual versions. Certificate metadata or hash matching alone is not a native
signature-validation result.

Run the portable tests without downloading any files:

```sh
python -B -m unittest discover -s scripts/asr-components/ct2-cpu/direct-crt -p 'test_*.py' -v
```

A Linux data-only audit independently extracted the CABs using the already
installed libarchive. Both selected DLLs matched the original installed VS
14.44.35211.0 DLLs byte for byte. They are AMD64 PE files. Their normal-import
closure requires only each other, the matching PBS-provided `vcruntime140.dll`
and `vcruntime140_1.dll`, and Windows UCRT API sets/kernel32. All four DLLs have
empty PE delay-import directories. This is static evidence, not a runtime test.
The native proof workflow establishes Windows extraction/signature behavior.

## Sources and licensing boundaries

- [Microsoft's version-pinned WinGet manifest](https://raw.githubusercontent.com/microsoft/winget-pkgs/master/manifests/m/Microsoft/VCRedist/2015%2B/x64/14.44.35211.0/Microsoft.VCRedist.2015%2B.x64.installer.yaml)
- [Microsoft runtime end-user terms](https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/)
- [Microsoft expand command](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/expand)
- [Microsoft app-local deployment guidance](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170)
- [WiX Burn format reference source](https://raw.githubusercontent.com/wixtoolset/wix3/develop/src/tools/wix/BurnCommon.cs)

The runtime EULA permits end-user installation/use and restricts onward sharing
and providing it with apps for others. It is not a developer redistribution
grant and does not explicitly address this automated extraction flow. Direct
user-to-Microsoft delivery avoids Luma hosting the bytes, but that is a factual
architectural distinction, not a legal clearance. Any later product setup must
provide the applicable terms before the user's explicit setup/acceptance action,
preserve notices, and pass independent supply-chain review. No terms are accepted
by this report-only extraction helper.
