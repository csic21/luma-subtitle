'use strict';
// Reuse the release validator; this bridge never queries or changes a release.
const { validateProof, validateLocks } = require('./ct2-cpu/publish.cjs');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', part => {
  input += part;
  if (Buffer.byteLength(input, 'utf8') > 8_000_000) throw new Error('Recipe proof input exceeds bound');
});
process.stdin.on('end', () => {
  try {
    const { proof, request, locks } = JSON.parse(input);
    validateLocks(locks.sources, locks.notices);
    validateProof(proof, request, locks);
    process.stdout.write('CPU_RECIPE_PROOF_OK\n');
  } catch (error) {
    process.stderr.write(String(error.message).slice(0, 2000) + '\n');
    process.exitCode = 1;
  }
});
