import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';

export const fixtureRoot = new URL('../spec/', import.meta.url);
export function inventory() {
  const files = new Set();
  function walk(dir, prefix) {
    for (const entry of readdirSync(dir, {withFileTypes: true})) {
      const name = prefix + entry.name;
      if (entry.isDirectory()) walk(new URL(entry.name + '/', dir), name + '/');
      else if (entry.name.endsWith('.astro')) files.add(name);
    }
  }
  walk(fixtureRoot, '');
  walk(new URL('../fixtures/', fixtureRoot), '../fixtures/');
  return files;
}
export function validateManifest(cases, files) {
  assert(Array.isArray(cases), 'manifest must be an array');
  const ids = new Set();
  const registered = new Set();
  for (const test of cases) {
    assert(typeof test.id === 'string' && /^[a-zA-Z0-9_]+$/.test(test.id), 'invalid fixture id');
    assert(!ids.has(test.id), `duplicate id: ${test.id}`);
    assert(!registered.has(test.file), `duplicate file: ${test.file}`);
    assert(files.has(test.file), `missing or invalid fixture: ${test.file}`);
    assert(['template', 'script', 'style'].includes(test.snapshot_group), `invalid snapshot group: ${test.id}`);
    assert(test.atlas && test.compiler, `missing expectations: ${test.id}`);
    ids.add(test.id);
    registered.add(test.file);
  }
  assert.deepEqual([...registered].sort(), [...files].sort(), 'unregistered fixtures');
}
export function fixtures() {
  const cases = JSON.parse(readFileSync(new URL('manifest.json', fixtureRoot), 'utf8'));
  validateManifest(cases, inventory());
  return cases;
}
export const sourceFor = test => readFileSync(new URL(test.file, fixtureRoot), 'utf8');
