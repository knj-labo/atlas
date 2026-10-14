import assert from 'node:assert/strict';
import { Script } from 'node:vm';
import { fixtures, sourceFor } from './fixtures.mjs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parse, transform } from '@astrojs/compiler-rs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const manifest = fixtures();
const build = spawnSync('cargo', ['build', '--locked', '--example', 'spec-dump', '--features', 'serde'], {cwd: root, stdio: 'inherit'});
assert.equal(build.status, 0, 'build test-only facts serializer');
const binary = `${process.env.CARGO_TARGET_DIR ?? `${root}/target`}/debug/examples/spec-dump`;
const span = node => ({start: node.start, end: node.end});
const slice = (source, range) => Buffer.from(source).subarray(range.start, range.end).toString('utf8');
function elements(ast, source) {
  const result = [];
  function visit(node, parent = null) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {for (const child of node) visit(child, parent); return;}
    if (['AstroFrontmatter', 'AstroScript', 'Program'].includes(node.type)) return;
    if (['JSXElement', 'JSXFragment'].includes(node.type)) {
      const opening = node.openingElement ?? node.openingFragment;
      const closing = node.closingElement ?? node.closingFragment;
      const id = result.length;
      result.push({parent, name: opening.name ? slice(source, span(opening.name)) : '', span: span(node),
        opening_span: span(opening), name_span: opening.name ? span(opening.name) : null,
        closing_span: closing ? span(closing) : null,
        attributes: (opening.attributes ?? []).map(a => ({name: a.name ? slice(source, span(a.name)) : null,
          span: span(a), name_span: a.name ? span(a.name) : null, spread: a.type === 'JSXSpreadAttribute',
          value: a.type === 'JSXSpreadAttribute' ? {span: span(a), raw: slice(source, span(a))} :
            a.value ? {span: span(a.value), raw: slice(source, span(a.value))} : null}))});
      // Attribute expressions can themselves contain template elements.
      visit(opening.attributes, id);
      visit(node.children, id);
      return;
    }
    for (const [key, value] of Object.entries(node)) {
      if (!['name', 'closingElement', 'openingElement'].includes(key)) visit(value, parent);
    }
  }
  visit(ast.body);
  return result;
}
function projected(facts) {
  return facts.elements.map(e => ({parent: e.parent, name: e.name, span: e.span, opening_span: e.opening_span,
    name_span: e.name_span, closing_span: e.closing_span, attributes: e.attributes.map(a => ({name: a.name,
      span: a.span, name_span: a.name_span, spread: a.spread,
      value: a.value ? {span: a.value.span, raw: a.value.raw} : null}))}));
}
let passed = 0;
let differences = 0;
for (const test of manifest) {
  const source = sourceFor(test);
  const run = spawnSync(binary, {input: source, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024});
  assert.equal(run.status, 0, `${test.file}: serializer ${run.stderr}`);
  const result = JSON.parse(run.stdout);
  const official = parse(source);
  const evidence = test.compiler;
  assert.equal(official.diagnostics.some(d => d.severity === 'error'), evidence.parseError, `${test.id}: official parse result`);
  assert.equal('Err' in result, test.atlas.error, `${test.id}: atlas analysis result ${JSON.stringify(result.Err)}`);
  if (result.Ok) {
    if (!evidence.parseError) assert.deepEqual(projected(result.Ok), elements(official.ast, source), `${test.id}: template names, parents, attributes and UTF-8 positions`);
    assert.deepEqual(result.Ok.scripts.map(s => s.mode), test.atlas.modes, `${test.id}: script modes`);
  }
  const compiled = transform(source, {filename: test.file});
  if (Array.isArray(evidence.scripts)) assert.deepEqual(compiled.scripts, evidence.scripts, `${test.id}: extracted script types and contents`);
  else if ('scripts' in evidence) assert.equal(compiled.scripts.length, evidence.scripts, `${test.id}: extracted scripts`);
  if ('css' in evidence) assert.equal(compiled.css.length, evidence.css, `${test.id}: extracted styles`);
  if ('scriptVars' in evidence) assert.equal(compiled.code.includes('defineScriptVars'), evidence.scriptVars, `${test.id}: script helper`);
  if ('styleVars' in evidence) assert.equal(compiled.code.includes('defineStyleVars'), evidence.styleVars, `${test.id}: style helper`);
  if ('wrappers' in evidence) assert.equal((compiled.code.match(/\(function\(\)\{/g) ?? []).length, evidence.wrappers, `${test.id}: generated IIFEs`);
  if (evidence.difference === 'nested-script-extraction') {
    assert.equal(result.Ok.scripts.filter(s => s.bundled).length, 3);
    assert.equal((compiled.code.match(/\$\$renderScript\(\$\$result/g) ?? []).length, evidence.renderScripts);
    differences += 1;
  }
  if (evidence.grammar) {
    const {kind, code, valid} = evidence.grammar;
    let accepted;
    if (kind === 'module') {
      const check = spawnSync(process.execPath, ['--input-type=module', '--check'], {input: code, encoding: 'utf8'});
      assert.ifError(check.error);
      accepted = check.status === 0;
    } else {
      try { new Script(code); accepted = true; } catch (error) { assert(error instanceof SyntaxError); accepted = false; }
    }
    assert.equal(accepted, valid, `${test.id}: generated binding grammar`);
  }
  passed += 1;
  console.log(`PASS ${test.id}${evidence.difference ? ` (documented difference: ${evidence.difference})` : ''}`);
}
console.log(`${passed} compiler evidence cases passed; ${differences} documented extraction differences.`);
