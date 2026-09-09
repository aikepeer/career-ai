/* Career workspace: dependency-free progressive enhancement. */
(function(root) {
  'use strict';
  const FILTERS = ['flt-remote', 'flt-india', 'flt-abroad', 'flt-embedded', 'flt-ai', 'flt-software', 'flt-stem', 'flt-state-discovered', 'flt-state-filtered'];

  function safePostingUrl(value) {
    if (typeof value !== 'string' || !/^https?:\/\//i.test(value)) return '#';
    try { const url = new URL(value); return ['https:', 'http:'].includes(url.protocol) ? url.href : '#'; }
    catch (_) { return '#'; }
  }

  function normalizeSavedViews(value) {
    if (!Array.isArray(value)) return [];
    return value.filter(v => v && typeof v.name === 'string' && v.name.trim() && typeof v.query === 'string' && Array.isArray(v.filters))
      .slice(0, 12).map(v => ({ name: v.name.trim().slice(0, 40), query: v.query.slice(0, 200), filters: FILTERS.filter(id => v.filters.includes(id)) }));
  }

  if (typeof module !== 'undefined') module.exports = { normalizeSavedViews, safePostingUrl };
  if (!root.document) return;
  root.safePostingUrl = safePostingUrl;
  root.openProfileSetup = function() {
    root.switchTab('config');
    const form = document.getElementById('profile-import-form');
    if (form) {
      form.scrollIntoView({ block: 'center', behavior: 'instant' });
      document.getElementById('profile-resume-file')?.focus({ preventScroll: true });
    }
  };
  const $ = id => document.getElementById(id);
  const read = (key, fallback) => { try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch (_) { return fallback; } };
  function write(key, value) {
    try { localStorage.setItem(key, JSON.stringify(value)); return true; }
    catch (_) { announce('Browser storage is unavailable. Changes will last for this visit only.'); return false; }
  }
  function announce(message) { if ($('workspace-status')) $('workspace-status').textContent = message; }

  function init() {
    const date = $('workspace-date');
    if (date) date.textContent = new Intl.DateTimeFormat(undefined, { weekday: 'long', month: 'short', day: 'numeric' }).format(new Date());
    document.querySelectorAll('.tab-nav .nav-btn').forEach(button => {
      const tab = button.id.replace('tab-btn-', '');
      button.setAttribute('aria-controls', 'tab-' + tab);
      button.setAttribute('aria-current', button.classList.contains('active') ? 'page' : 'false');
    });
    initPalette();
    initSavedSearches();
    initActivity();
  }

  function initPalette() {
    const dialog = $('command-palette');
    const input = $('command-search');
    const results = $('command-results');
    if (!dialog || !input || !results) return;
    const commands = [
      ['Your overview', 'Daily focus, pipeline and weekly progress', 'funnel'],
      ['Explore opportunities', 'Search and filter all discovered jobs', 'explorer'],
      ['Review applications', 'Tailored resumes, drafts and next actions', 'actions'],
      ['Profile and sources', 'Import your resume and configure your search', 'config'],
      ['Insights and follow-ups', 'Outcomes, source quality and reminders', 'analytics'],
      ['Ask your assistant', 'Get help with your search and interviews', 'chat'],
      ['Activity log', 'Inspect recent pipeline events', 'events'],
      ['CLI commands', 'Find the command for a task', 'commands'],
      ['Bounty directory', 'Explore the curated platform directory', 'bounties'],
    ];
    let previousFocus;
    let destinationFocus;
    function render() {
      results.replaceChildren();
      const query = input.value.trim().toLowerCase();
      const matches = commands.filter(c => c.join(' ').toLowerCase().includes(query));
      matches.forEach(([title, description, tab]) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'command-result';
        const label = document.createElement('strong');
        label.textContent = title;
        const hint = document.createElement('span');
        hint.textContent = description;
        button.append(label, hint);
        button.addEventListener('click', () => {
          destinationFocus = $('tab-btn-' + tab);
          dialog.close();
          root.switchTab(tab);
        });
        results.append(button);
      });
      if (!matches.length) {
        const empty = document.createElement('p');
        empty.className = 'command-empty';
        empty.textContent = 'No matching pages. Try “jobs”, “profile” or “follow-ups”.';
        results.append(empty);
      }
      $('command-count').textContent = matches.length + ' destinations';
    }
    function open() {
      if (dialog.open) return;
      previousFocus = document.activeElement;
      destinationFocus = null;
      input.value = '';
      render();
      dialog.showModal();
      input.focus();
    }
    $('command-trigger')?.addEventListener('click', open);
    $('command-close')?.addEventListener('click', () => dialog.close());
    dialog.addEventListener('close', () => (destinationFocus || previousFocus)?.focus());
    dialog.addEventListener('click', event => { if (event.target === dialog) dialog.close(); });
    input.addEventListener('input', render);
    dialog.addEventListener('keydown', event => {
      const buttons = [...results.querySelectorAll('button')];
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        const index = buttons.indexOf(document.activeElement);
        const next = event.key === 'ArrowDown' ? index + 1 : (index <= 0 ? buttons.length - 1 : index - 1);
        buttons[next % buttons.length]?.focus();
      } else if (event.key === 'Enter' && document.activeElement === input) {
        event.preventDefault(); buttons[0]?.click();
      }
    });
    document.addEventListener('keydown', event => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); open(); }
    });
  }

  function initSavedSearches() {
    const container = $('saved-searches');
    if (!container) return;
    let saved = normalizeSavedViews(read('careerai-saved-searches-v1', []));
    const form = $('save-search-form');
    const name = $('saved-search-name');
    function apply(view) {
      $('explorer-search-input').value = view.query;
      FILTERS.forEach(id => { if ($(id)) $(id).checked = view.filters.includes(id); });
      root.filterExplorerTable();
      announce('Search applied: ' + view.name);
    }
    function render() {
      container.replaceChildren();
      saved.forEach((view, index) => {
        const group = document.createElement('span');
        group.className = 'saved-search';
        const button = document.createElement('button');
        button.type = 'button'; button.textContent = view.name;
        button.addEventListener('click', () => apply(view));
        const remove = document.createElement('button');
        remove.type = 'button'; remove.textContent = '×';
        remove.setAttribute('aria-label', 'Delete saved search ' + view.name);
        remove.addEventListener('click', () => {
          saved.splice(index, 1); write('careerai-saved-searches-v1', saved); render();
          $('save-search-toggle').focus(); announce('Saved search deleted.');
        });
        group.append(button, remove); container.append(group);
      });
      if (!saved.length) container.textContent = 'Keep your favorite filters one click away.';
    }
    $('save-search-toggle').addEventListener('click', () => { form.hidden = !form.hidden; if (!form.hidden) name.focus(); });
    form.addEventListener('submit', event => {
      event.preventDefault();
      const label = name.value.trim();
      if (!label) { name.focus(); return; }
      const existing = saved.findIndex(v => v.name.toLowerCase() === label.toLowerCase());
      if (saved.length >= 12 && existing === -1) { announce('You can save 12 searches. Delete one to make room.'); return; }
      const view = { name: label.slice(0, 40), query: $('explorer-search-input').value.slice(0, 200), filters: FILTERS.filter(id => $(id)?.checked) };
      if (existing >= 0) saved[existing] = view; else saved.push(view);
      const persisted = write('careerai-saved-searches-v1', saved);
      render(); form.hidden = true; name.value = ''; $('save-search-toggle').focus();
      if (persisted) announce('Search saved in this browser.');
    });
    document.querySelectorAll('[data-search-preset]').forEach(button => {
      button.addEventListener('click', () => {
        apply({ name: button.textContent.trim(), query: '', filters: button.dataset.searchPreset === 'remote' ? ['flt-remote', 'flt-state-discovered'] : ['flt-state-discovered', 'flt-state-filtered'] });
      });
    });
    render();
  }

  function initActivity() {
    const goalInput = $('weekly-goal');
    if (!goalInput) return;
    const storedGoal = read('careerai-weekly-goal-v1', 10);
    let goal = Number.isInteger(storedGoal) && storedGoal > 0 && storedGoal <= 100 ? storedGoal : 10;
    let submitted = null;
    goalInput.value = goal;
    function updateGoal() {
      $('weekly-goal-label').textContent = goal;
      const progress = $('weekly-progress');
      progress.max = goal;
      if (submitted !== null) {
        $('weekly-submitted').textContent = submitted;
        progress.value = Math.min(submitted, goal);
        $('weekly-caption').textContent = submitted >= goal ? 'Target reached. Make time for your next conversation.' : (goal - submitted) + ' to your target · resets Monday, UTC';
      }
    }
    goalInput.addEventListener('change', () => {
      const value = Number(goalInput.value);
      if (!Number.isInteger(value) || value < 1 || value > 100) { goalInput.value = goal; announce('Choose a weekly target between 1 and 100.'); return; }
      goal = value; write('careerai-weekly-goal-v1', goal); updateGoal();
    });
    async function load() {
      $('activity-retry').hidden = true;
      try {
        const response = await fetch('/api/v1/activity');
        if (!response.ok) throw new Error('Activity unavailable');
        const data = await response.json();
        if (!Array.isArray(data.days) || !Number.isInteger(data.submitted_this_week)) throw new Error('Invalid activity');
        submitted = data.submitted_this_week;
        updateGoal();
        const chart = $('activity-chart');
        chart.replaceChildren();
        const max = Math.max(1, ...data.days.map(day => day.submitted));
        data.days.forEach(day => {
          const item = document.createElement('li');
          const count = document.createElement('span'); count.className = 'activity-count'; count.textContent = day.submitted;
          const bar = document.createElement('span'); bar.className = 'activity-bar'; bar.style.setProperty('--bar-size', Math.max(3, day.submitted / max * 100) + '%'); bar.setAttribute('aria-hidden', 'true');
          const label = document.createElement('span'); label.className = 'activity-label';
          label.textContent = new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' }).format(new Date(day.date + 'T12:00:00Z'));
          item.setAttribute('aria-label', day.date + ': ' + day.submitted + ' applications sent, ' + day.responded + ' responses');
          item.title = item.getAttribute('aria-label');
          item.append(count, bar, label); chart.append(item);
        });
        $('activity-status').textContent = 'Last 7 days · actual submissions · UTC';
      } catch (_) {
        $('weekly-caption').textContent = 'Activity could not be loaded.';
        $('activity-status').textContent = 'Your target is saved; activity is unavailable.';
        $('activity-retry').hidden = false;
      }
    }
    $('activity-retry').addEventListener('click', load);
    updateGoal(); load();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
})(typeof window === 'undefined' ? globalThis : window);
