import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { stringifyForScript } from 'astro/runtime/server/escape.js';
import { serializeProps } from 'astro/runtime/server/serialize.js';

const root = fileURLToPath(new URL('../../', import.meta.url));
const require = createRequire(import.meta.url);
const astro = JSON.parse(readFileSync(require.resolve('astro/package.json'), 'utf8'));
assert.equal(astro.version, '7.0.6', 'runtime oracle must match the pinned transfer contract');

const build = spawnSync('cargo', ['build', '--locked', '--example', 'transfer-dump', '--message-format=json'], {
  cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024,
});
assert.ifError(build.error);
assert.equal(build.status, 0, `build test-only transfer projection: ${build.stderr}`);
const binary = build.stdout.trim().split('\n').map(line => JSON.parse(line))
  .find(message => message.reason === 'compiler-artifact' && message.target.name === 'transfer-dump')?.executable;
assert.ok(binary, 'Cargo reports the executable path, including custom target directories');

function checkAtlas({ expression, declarations = '' }, expected = ['Supported', 'Supported']) {
  const source = `---\nimport View from './View';\n${declarations}\n---\n` +
    `<script define:vars={{ value: ${expression} }}></script>\n` +
    `<View client:load value={${expression}} />`;
  const run = spawnSync(binary, { cwd: root, input: source, encoding: 'utf8' });
  assert.ifError(run.error);
  assert.equal(run.status, 0, `analyze ${expression}: ${run.stderr}`);
  assert.deepEqual(JSON.parse(run.stdout), ['JsonScript', 'AstroProps'].map((method, i) => ({
    method, inputs: [{ name: 'value', status: expected[i] }],
  })));
}

// Each factory is trusted fixture code. The analyzed expressions are never evaluated.
// Tags and payloads below are explicit expectations, not a second serializer.
const cases = [
  { name: 'null', expression: 'null', create: () => null, json: null, props: [0, null] },
  { name: 'empty string', expression: "''", create: () => '', json: '', props: [0, ''] },
  { name: 'true', expression: 'true', create: () => true, json: true, props: [0, true] },
  { name: 'false', expression: 'false', create: () => false, json: false, props: [0, false] },
  { name: 'negative fraction', expression: '-12.5', create: () => -12.5, json: -12.5, props: [0, -12.5] },
  { name: 'unary plus', expression: '+2', create: () => 2, json: 2, props: [0, 2] },
  { name: 'negative zero becomes zero', expression: '-0', create: () => -0, json: 0, props: [0, 0] },
  { name: 'hexadecimal', expression: '0xff', create: () => 255, json: 255, props: [0, 255] },
  { name: 'octal', expression: '0o17', create: () => 15, json: 15, props: [0, 15] },
  { name: 'binary', expression: '0b1010', create: () => 10, json: 10, props: [0, 10] },
  { name: 'numeric separators', expression: '1_000.25', create: () => 1000.25, json: 1000.25, props: [0, 1000.25] },
  { name: 'exponent', expression: '1.25e3', create: () => 1250, json: 1250, props: [0, 1250] },
  { name: 'large finite number', expression: '1e21', create: () => 1e21, json: 1e21, props: [0, 1e21] },
  {
    name: 'Unicode and closing script tags', expression: "'</script><script>日本語😀\\u2028\\u2029'",
    create: () => '</script><script>日本語😀\u2028\u2029',
    json: '</script><script>日本語😀\u2028\u2029', props: [0, '</script><script>日本語😀\u2028\u2029'],
    script: '"\\u003c/script>\\u003cscript>日本語😀\u2028\u2029"',
  },
  { name: 'empty array', expression: '[]', create: () => [], json: [], props: [1, []] },
  { name: 'empty object', expression: '{}', create: () => ({}), json: {}, props: [0, {}] },
  {
    name: 'computed prototype data key', expression: "{ ['__proto__']: 1 }",
    create: () => ({ ['__proto__']: 1 }), json: { ['__proto__']: 1 }, props: [0, { ['__proto__']: [0, 1] }],
  },
  {
    name: 'nested arrays and objects', expression: "{ text: '</script>😀', values: [null, false, -0, { amount: 16 }] }",
    create: () => ({ text: '</script>😀', values: [null, false, -0, { amount: 16 }] }),
    json: { text: '</script>😀', values: [null, false, 0, { amount: 16 }] },
    props: [0, { text: [0, '</script>😀'], values: [1, [[0, null], [0, false], [0, 0], [0, { amount: [0, 16] }]]] }],
  },
  { name: 'BigInt', expression: '1n', create: () => 1n, jsonError: true, props: [6, '1'], status: ['Unsupported', 'Supported'] },
  { name: 'negative hexadecimal BigInt', expression: '-0xfn', create: () => -15n, jsonError: true, props: [6, '-15'], status: ['Unsupported', 'Supported'] },
  { name: 'large BigInt', expression: '9_007_199_254_740_993n', create: () => 9007199254740993n, jsonError: true, props: [6, '9007199254740993'], status: ['Unsupported', 'Supported'] },
  {
    name: 'nested BigInt', expression: '[1n, { negative: -2n }]',
    create: () => [1n, { negative: -2n }], jsonError: true,
    props: [1, [[6, '1'], [0, { negative: [6, '-2'] }]]], status: ['Unsupported', 'Supported'],
  },
  {
    name: 'primitive const aliases', expression: 'alias', declarations: "const base = '😀'; const alias = base;",
    create: () => '😀', json: '😀', props: [0, '😀'],
  },
  {
    name: 'BigInt const aliases', expression: 'alias', declarations: 'const base = -15n; const alias = base;',
    create: () => -15n, jsonError: true, props: [6, '-15'], status: ['Unsupported', 'Supported'],
  },
];

for (const fixture of cases) {
  test(`Astro 7.0.6 preserves ${fixture.name}`, () => {
    checkAtlas(fixture, fixture.status);
    if (fixture.jsonError) {
      assert.throws(() => stringifyForScript(fixture.create()), TypeError);
    } else {
      const script = stringifyForScript(fixture.create());
      assert.equal(typeof script, 'string');
      assert.equal(script.includes('<'), false, 'script embedding escapes every less-than character');
      assert.deepEqual(JSON.parse(script), fixture.json);
      if (fixture.script !== undefined) assert.equal(script, fixture.script);
    }
    assert.deepEqual(JSON.parse(serializeProps({ value: fixture.create() })), { value: fixture.props });
  });
}

const functions = [
  { name: 'arrow', expression: '() => 1', create: () => () => 1 },
  { name: 'function', expression: 'function () {}', create: () => function () {} },
  { name: 'generator', expression: 'function* () {}', create: () => function* () {} },
  { name: 'async arrow', expression: 'async () => 1', create: () => async () => 1 },
];
for (const fixture of functions) {
  test(`function loss: ${fixture.name}`, () => {
    checkAtlas(fixture, ['Unsupported', 'Unsupported']);
    assert.equal(stringifyForScript(fixture.create()), undefined, 'a top-level function has no JSON value');
    assert.deepEqual(JSON.parse(serializeProps({ value: fixture.create() })), { value: [0, null] });
  });
}

for (const fixture of [
  { name: 'array member', expression: '[() => 1]', create: () => [() => 1], json: [null], props: [1, [[0, null]]] },
  {
    name: 'object member', expression: '{ run: () => 1, retained: 2 }', create: () => ({ run: () => 1, retained: 2 }),
    json: { retained: 2 }, props: [0, { run: [0, null], retained: [0, 2] }],
  },
  { name: 'object method', expression: '{ run() {} }', create: () => ({ run() {} }), json: {}, props: [0, { run: [0, null] }] },
]) {
  test(`function loss: ${fixture.name}`, () => {
    checkAtlas(fixture, ['Unsupported', 'Unsupported']);
    assert.deepEqual(JSON.parse(stringifyForScript(fixture.create())), fixture.json);
    assert.deepEqual(JSON.parse(serializeProps({ value: fixture.create() })), { value: fixture.props });
  });
}

for (const fixture of [
  {
    name: 'const object', expression: 'value', declarations: 'const value = { count: 1 };',
    create: () => ({ count: 1 }), json: { count: 1 }, props: [0, { count: [0, 1] }],
  },
  { name: 'const array', expression: 'value', declarations: 'const value = [1];', create: () => [1], json: [1], props: [1, [[0, 1]]] },
  { name: 'call result', expression: 'read()', create: () => 42, json: 42, props: [0, 42] },
  {
    name: 'const call result', expression: 'value', declarations: 'const value = read();',
    create: () => ({ count: 1 }), json: { count: 1 }, props: [0, { count: [0, 1] }],
  },
  { name: 'undefined reference', expression: 'undefined', create: () => undefined, jsonUndefined: true, props: [0] },
]) {
  test(`Atlas keeps ${fixture.name} unknown despite runtime support`, () => {
    checkAtlas(fixture, ['Unknown', 'Unknown']);
    if (fixture.jsonUndefined) assert.equal(stringifyForScript(fixture.create()), undefined);
    else assert.deepEqual(JSON.parse(stringifyForScript(fixture.create())), fixture.json);
    assert.deepEqual(JSON.parse(serializeProps({ value: fixture.create() })), { value: fixture.props });
  });
}
