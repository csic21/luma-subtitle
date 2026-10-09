# Direct Microsoft CRT package audit

This directory contains a **report-only proof**, not app integration or a grant
of redistribution rights. The downloaded installer and its DLLs are never run.
There are no global installation, registry, PATH, administrator, or license
acceptance steps. Commit and upload only code and JSON metadata, never the
Microsoft installer, cabinet, DLL, or runtime bytes.

## Exact input and extraction

`package.lock.json` is the compact, data-only identity/mapping shared with the
reviewed fixed Rust integration. Its duplicated constants are checked against the audit
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
  English EULA identifies `Cpp_2015-2022_ENU.1033`; the Chinese EULA identifies
  `Cpp_2015-2022_CHS.2052`.

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

The registry provenance is pinned to WinGet commit
`3956ea1dc1086c62fa6d2d05484952caee010646`, Git blob
`6cdab74037d685cf44e9f811043cb9fc3a44cb80`. Its original CRLF manifest is 907
bytes with SHA-256
`de0d58bf1227acee24c6a920f31fc0f3f922d263725ccfec2bc5fe7553875f60`.
These identities were checked through the GitHub connector at the exact commit;
the computed Git blob SHA agrees. No mutable branch ref is needed for provenance.

- [Microsoft's version-pinned WinGet manifest](https://raw.githubusercontent.com/microsoft/winget-pkgs/3956ea1dc1086c62fa6d2d05484952caee010646/manifests/m/Microsoft/VCRedist/2015%2B/x64/14.44.35211.0/Microsoft.VCRedist.2015%2B.x64.installer.yaml)
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

## Native evidence and application boundary

The independent Windows [package audit job](https://github.com/csic21/luma-subtitle/actions/runs/37931498892/job/113823077314), on source commit
`c6dea1008caa66801bd6b3d7651059dd9df06002`, verified the installer and both
extracted DLLs with native Authenticode `Valid`; all three report version
14.44.35211.0. Both preserved RTF hashes also match. Only JSON metadata was
uploaded. The lock records separate installer and DLL leaf-certificate DER
SHA-256 identities; these are compared with the signer returned by successful
Windows trust verification, not an arbitrary embedded certificate.

The application uses an owned early-entrypoint child for synchronous
`WinVerifyTrust`, with a held read-only file handle, exact whole-file SHA-256,
no UI, chain-excluding-root revocation checks, MOTW policy, MD2/MD4 disabled,
zero-only success, and unconditional trust-state close. Ordinary user-triggered
Install and Repair allow normal Windows certificate-chain/revocation network
validation. A cached package does not imply network-free trust validation.
Cache-only is a distinct internal proof mode, with no automatic fallback after
a failed trust result. Helpers are time-bounded, cancellable and reaped before
their private staging and setup lease are released.

Full English and Simplified Chinese UTF-8 terms, exact raw RTF hashes, locale
EULA IDs and this contract's bytes are bound into the setup plan before any
download. The two original RTF notices are retained under
`licenses/msvc-14.44.35211-x64/`. All CRT destinations are reserved before pip
and reverified afterwards. The Rust implementation and actual Windows-subsystem
app helper still require their separate native proof before runtime activation;
the earlier package audit alone does not establish that application behavior.
