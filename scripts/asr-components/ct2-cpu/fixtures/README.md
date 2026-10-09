# Pinned source regression fixture

These are exact, unmodified source files from
OpenNMT/CTranslate2 commit `d44d2d069eb88c7b7804da864c10c201501cb4a9`,
under `ctranslate2/`:

- `src/ops/conv1d_cpu.cc` (10,461 bytes; SHA-256
  `40649222865eb74df6be6433c7b58466da9670827342ebeba67c0577691e1637`)
- `src/cpu/primitives.cc` (43,879 bytes; SHA-256
  `b1385185fab673357e482604e06cbaed57017580b1cd4e931b5361ba50f864a6`)

They retain the upstream MIT license in `../licenses/ctranslate2/LICENSE.txt`.
The local attributes preserve exact source bytes on Windows checkouts.

The cheap regression verifies that the reviewed oneDNN primitive selection
includes convolution and reorder plus the retained MATMUL selection. These are
the only files using oneDNN APIs across pinned CT2 `src/` and `include/`:
`primitives.cc` calls three low-level float/int8 GEMM C APIs; `conv1d_cpu.cc`
uses convolution/reorder and engine/memory scaffolding. CT2 applies activation
itself after convolution, without independent or fused oneDNN post-ops.
The builder scans the complete freshly extracted library source trees and checks
the exact file/API inventory before compiling. Any new call site or changed
source fails for review. These fixtures contain source code, not model weights.
