// No npm dependencies: node --test crates/careerai-dashboard/tests/workspace_ui.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const template = fs.readFileSync(path.join(__dirname, '../templates/index.tera'), 'utf8');

test('a disabled refresh interval never starts polling', () => {
  const start = template.indexOf('// Smart Countdown Timer Script');
  const script = template.slice(start, template.indexOf('</script>', start))
    .replace('{{ refresh_seconds }}', '0');
  let timers = 0;
  vm.runInNewContext(script, {
    document: { getElementById: () => ({}) },
    setInterval: () => { timers++; },
  });
  assert.equal(timers, 0);
});

test('saved searches validate stored data and preserve meaningful filters', () => {
  const ui = require('../static/workspace.js');
  const views = ui.normalizeSavedViews([
    { name: 'Remote Rust', query: 'rust', filters: ['flt-remote', 'flt-state-discovered', 'not-a-filter'] },
    null, { name: '', query: 'x' }, { name: 'Broken', query: {} },
  ]);
  assert.deepEqual(views, [
    { name: 'Remote Rust', query: 'rust', filters: ['flt-remote', 'flt-state-discovered'] },
  ]);
  assert.deepEqual(ui.normalizeSavedViews({ bad: true }), []);
  assert.equal(ui.normalizeSavedViews(Array(30).fill({ name: 'a', query: '', filters: [] })).length, 12);
});

test('posting links reject script and data schemes while preserving https links', () => {
  const { safePostingUrl } = require('../static/workspace.js');
  for (const value of ['javascript:alert(1)', 'data:text/html,hello', '//evil.test', 'java\nscript:alert(1)', null]) {
    assert.equal(safePostingUrl(value), '#');
  }
  assert.equal(safePostingUrl('https://example.com/jobs?a=1&b=2'), 'https://example.com/jobs?a=1&b=2');
});

test('service worker only handles public static GET assets', () => {
  const listeners = {};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../static/sw.js'), 'utf8'), {
    URL, self: { location: { origin: 'http://localhost:8787' }, addEventListener: (name, fn) => { listeners[name] = fn; } },
    caches: { match: () => Promise.resolve(undefined) },
    fetch: () => Promise.resolve({ ok: false }),
  });
  for (const [pathname, method] of [['/', 'GET'], ['/api/v1/artifacts/download?path=resume.pdf', 'GET'], ['/healthz', 'GET'], ['/static/workspace.js', 'POST']]) {
    let handled = false;
    listeners.fetch({ request: { url: 'http://localhost:8787' + pathname, method }, respondWith: () => { handled = true; } });
    assert.equal(handled, false, pathname + ' must go directly to the server');
  }
});

test('page links work when browser storage is unavailable', () => {
  const start = template.indexOf('function restoreWorkspaceTab()');
  const script = template.slice(start, template.indexOf('// State Restoration', start));
  const destinations = [];
  const context = {
    URLSearchParams,
    window: { location: { search: '?tab=explorer' } },
    localStorage: { getItem: () => { throw new Error('Storage disabled'); } },
    switchTab: tab => destinations.push(tab),
  };
  vm.runInNewContext(script + '\nrestoreWorkspaceTab();', context);
  assert.deepEqual(destinations, ['explorer']);
  context.window.location.search = '?tab=unknown';
  vm.runInNewContext('restoreWorkspaceTab();', context);
  assert.deepEqual(destinations, ['explorer']);
});

test('failed explorer refresh preserves results and its retry survives filtering', async () => {
  const filters = require('../static/workspace.js').normalizeSavedViews([
    { name: 'All', query: '', filters: ['flt-state-discovered', 'flt-state-filtered'] },
  ])[0].filters;
  const rows = [{ style: {}, getAttribute: key => key === 'data-state' ? 'discovered' : '' }];
  function element() {
    return { hidden: true, value: '', checked: false, textContent: '', children: [],
      appendChild(child) { this.children.push(child); },
      replaceChildren() { rows.length = 0; },
      remove() {},
    };
  }
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  filters.forEach(id => { get(id).checked = true; });
  let requests = 0;
  const context = vm.createContext({
    document: { getElementById: get, querySelectorAll: () => rows, createElement: element },
    window: {}, _explorerLoaded: true, _allExplorerItems: [{}],
    fetch: async () => { requests++; return { ok: false }; },
    renderExplorerRowsChunk: () => {},
  });
  const filterStart = template.indexOf('function filterExplorerTable()');
  const loadStart = template.indexOf('var _explorerGeneration =');
  vm.runInContext(template.slice(filterStart, template.indexOf('function resetExplorerFilters()', filterStart)), context);
  vm.runInContext(template.slice(loadStart, template.indexOf('function renderExplorerRowsChunk(', loadStart)), context);
  await context.loadExplorerListings();
  assert.equal(rows.length, 1, 'a failed refresh must preserve the previous rows');
  assert.equal(get('explorer-load-error').hidden, false);
  context.filterExplorerTable();
  assert.equal(get('explorer-load-error').hidden, false, 'filtering must not hide the failure');
  assert.match(get('explorer-result-status').textContent, /1 matching/);
  const retry = get('explorer-load-error').children[0];
  context.fetch = async () => { requests++; return { ok: true, json: async () => [] }; };
  await retry.onclick();
  assert.equal(requests, 2);
  assert.equal(get('explorer-load-error').hidden, true);
  assert.equal(rows.length, 0, 'a successful refresh must replace stale rows, even with zero results');
});
