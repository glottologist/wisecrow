// TypeScript guideline compliant 2026-09-10
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const indexHtml = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const scriptMatch = indexHtml.match(/<script>([\s\S]*?)<\/script>/u);
if (scriptMatch === null || scriptMatch[1] === undefined) {
  throw new Error('index.html must contain an inline theme bootstrap');
}
const bootstrap = scriptMatch[1];

function runBootstrap({
  saved = null,
  prefersDark = false,
  storageUnavailable = false,
  mediaUnavailable = false,
} = {}) {
  const dataset = {};
  const context = {
    document: { documentElement: { dataset } },
    window: {
      localStorage: {
        getItem(key) {
          assert.equal(key, 'wisecrow-theme');
          if (storageUnavailable) throw new Error('storage unavailable');
          return saved;
        },
      },
      matchMedia(query) {
        assert.equal(query, '(prefers-color-scheme: dark)');
        if (mediaUnavailable) throw new Error('media query unavailable');
        return { matches: prefersDark };
      },
    },
  };

  vm.runInNewContext(bootstrap, context);
  return dataset.theme;
}

const preferenceCases = [
  ['saved light overrides dark device', { saved: 'light', prefersDark: true }, 'light'],
  ['saved dark overrides light device', { saved: 'dark' }, 'dark'],
  ['missing choice follows light device', {}, 'light'],
  ['missing choice follows dark device', { prefersDark: true }, 'dark'],
  ['invalid choice follows device', { saved: 'sepia', prefersDark: true }, 'dark'],
  ['blocked storage still follows device', { storageUnavailable: true }, 'light'],
  [
    'unavailable storage and media use dark fallback',
    { storageUnavailable: true, mediaUnavailable: true },
    'dark',
  ],
];

for (const [name, options, expected] of preferenceCases) {
  test(name, () => assert.equal(runBootstrap(options), expected));
}

test('bootstrap remains in the static head before the Dioxus mount', () => {
  const scriptEnd = indexHtml.indexOf('</script>');
  const headEnd = indexHtml.indexOf('</head>');
  const mount = indexHtml.indexOf('<div id="main"></div>');

  assert.ok(scriptEnd >= 0 && scriptEnd < headEnd && headEnd < mount);
});
