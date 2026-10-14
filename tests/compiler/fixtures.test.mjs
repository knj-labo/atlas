import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fixtures, inventory, validateManifest } from './fixtures.mjs';

const base = fixtures();
const files = inventory();
for (const [name, mutate] of [
  ['duplicate ids', cases => cases.push(structuredClone(cases[0]))],
  ['duplicate paths', cases => { cases[1].file = cases[0].file; }],
  ['missing files', cases => { cases[0].file = 'missing.astro'; }],
  ['unregistered files', cases => cases.pop()],
]) {
  test(`catalog rejects ${name}`, () => {
    const cases = structuredClone(base);
    mutate(cases);
    assert.throws(() => validateManifest(cases, files));
  });
}
