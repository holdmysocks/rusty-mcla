// Launcher shell: tabs, sub-tabs, settings binding, theme picker, window chrome. No network and no install logic yet.
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);

  // The keyboard table (the game's 32 actions, labels act.<ID>) is drawn by controls.js from the save; the pad table stays a read-only placeholder.
  const PAD_ACTIONS = ['accelerate', 'brake', 'steer', 'handbrake', 'nitrous', 'weight', 'ability', 'next_ability', 'headlights', 'horn', 'camera', 'look_back'];
  // Folders tab: id, has a second button (locale key suffix) or none.
  const FOLDERS = [['install', 'change'], ['data', 'rebuild'], ['saves', null], ['backups', 'change'], ['logs', null], ['screenshots', 'change'], ['tools', 'use_mine']];

  const TABS = ['home', 'install', 'mods', 'music', 'controls', 'settings', 'support'];
  const LOCKED = ['mods', 'music']; // later features: never reachable
  const SETUP_FIRST = ['install', 'controls']; // disabled on first run, as in the design
  const SUB_DEFAULT = { controls: 'kbd', settings: 'general' };

  const state = { boot: null, settings: null, installed: false, setupStarted: false, tab: 'home', sub: Object.assign({}, SUB_DEFAULT), pageId: null };
  const dev = new URLSearchParams(location.search);

  // ---------- settings ----------
  const getPath = (o, path) => path.split('.').reduce((a, k) => (a == null ? a : a[k]), o);
  const patchFor = (path, value) => path.split('.').reduceRight((acc, k) => ({ [k]: acc }), value);

  async function setSetting(path, value) {
    try {
      state.settings = await Bridge.saveSettings(patchFor(path, value));
    } catch (e) {
      Bridge.log('error', 'could not save ' + path + ': ' + e);
      toast(t('toast.save_failed'), true);
    }
    return state.settings;
  }

  // ---------- toast ----------
  let toastTimer = null;
  // `action` = { label, fn } adds a link to the toast ("Show in folder"); such a toast stays longer
  function toast(text, warn, action) {
    const el = $('#toast');
    $('#toast-text', el).textContent = text;
    el.classList.toggle('warn', !!warn);
    const a = $('#toast-action', el);
    a.hidden = !action;
    a.onclick = null;
    if (action) {
      a.textContent = action.label;
      a.onclick = (e) => { e.preventDefault(); el.hidden = true; action.fn(); };
    }
    el.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { el.hidden = true; }, action ? 9000 : 3200);
  }

  // ---------- pages and tabs ----------
  function pageFor(tab) {
    if (tab === 'home') return state.installed ? 'home' : 'firstrun';
    return tab;
  }

  // the install pipeline finished (or an install was found): Home becomes the real page, Install and Controls stay open
  function setInstalled(v) {
    state.installed = !!v;
    refreshRail();
    if (state.tab === 'home') showPage(pageFor('home'));
    if (window.Install) Install.render();
  }

  function tabAvailable(tab) {
    if (LOCKED.includes(tab)) return false;
    if (SETUP_FIRST.includes(tab) && !state.installed && !state.setupStarted) return false;
    return true;
  }

  function refreshRail() {
    $$('.tab').forEach((b) => {
      const tab = b.dataset.tab;
      const ok = tabAvailable(tab);
      b.setAttribute('aria-disabled', ok ? 'false' : 'true');
      b.dataset.lock = ok ? '' : LOCKED.includes(tab) ? 'later' : 'setup';
      if (tab === state.tab) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current');
    });
  }

  function showPage(pageId) {
    state.pageId = pageId;
    $$('.page').forEach((p) => { p.hidden = p.dataset.page !== pageId; });
    const page = $('.page[data-page="' + pageId + '"]');
    if (!page) return;
    $('#head-title').textContent = t(page.dataset.titleKey);
    $('#head-title').classList.toggle('wordmark', page.dataset.head === 'brand');
    $('#head-tag').hidden = page.dataset.head !== 'brand';
    $('#head-flag').hidden = page.dataset.head !== 'preview';
    $('#head-status').hidden = pageId !== 'home';
    $('#foot').hidden = !page.hasAttribute('data-foot');
    const subs = $$('[data-sub-page]', page);
    if (subs.length) showSub(page, state.sub[page.dataset.page.replace('-preview', '')] || SUB_DEFAULT[page.dataset.page]);
  }

  function showTab(tab) {
    if (!tabAvailable(tab)) return;
    state.tab = tab;
    refreshRail();
    showPage(pageFor(tab));
  }

  function showSub(page, sub) {
    const name = page.dataset.page;
    if (!sub) return;
    state.sub[name] = sub;
    $$('.pill[data-sub]', page).forEach((p) => p.setAttribute('aria-selected', p.dataset.sub === sub ? 'true' : 'false'));
    $$('[data-sub-page]', page).forEach((s) => { s.hidden = s.dataset.subPage !== sub; });
  }

  // ---------- generated markup (rows and cards that repeat) ----------
  function el(tag, cls, attrs) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]);
    return e;
  }
  const span = (cls, key) => { const s = el('span', cls); s.setAttribute('data-i18n', key); return s; };

  function buildKeyRows() {
    const pad = $('#pad-rows');
    PAD_ACTIONS.forEach((id) => {
      const row = el('div', 'tbl-row one');
      row.append(span('act', 'pad.' + id), span('keycap', 'ctl.button'));
      pad.append(row);
    });
  }

  function buildThemeGrid() {
    const grid = $('#theme-grid');
    Themes.list.forEach((th) => {
      const b = el('button', 'theme', { role: 'radio', 'data-theme-id': th.id, 'aria-checked': 'false' });
      const prev = el('div', 'tp');
      const parts = th.id === 'system' ? [['steel', 'left'], ['alucard', 'right']] : [[th.id, '']];
      parts.forEach(([id, side]) => {
        const p = el('div', 'tp' + (side ? ' half ' + side : ''), { 'data-theme': id });
        ['t-rail', 't-hero', 't-play', 't-card', 't-l1', 't-l2', 't-l3', 't-c1', 't-c2', 't-c3'].forEach((c) => p.append(el('i', c)));
        prev.append(p);
      });
      const tick = el('div', 'tick');
      tick.innerHTML = '<svg class="ic"><use href="#i-check"/></svg>';
      prev.append(tick);
      const meta = el('div', 'meta');
      const top = el('div', 'top');
      top.append(span('name', 'theme.' + th.id + '.name'), span('mode', 'mode.' + th.mode));
      meta.append(top, span('desc', 'theme.' + th.id + '.desc'));
      meta.lastChild.style.display = 'block';
      b.append(prev, meta);
      b.addEventListener('click', () => selectTheme(th.id));
      grid.append(b);
    });
  }

  function buildFolderRows() {
    const host = $('#folder-rows');
    FOLDERS.forEach(([id, btn]) => {
      const row = el('div', 'frow');
      const name = el('div');
      name.append(span('ft', 'f.' + id), span('fd', 'f.' + id + '_d'));
      const path = el('div', 'field', { 'data-folder-path': id });
      const second = btn ? el('button', 'btn', { 'data-later': '' }) : el('span');
      if (btn) second.setAttribute('data-i18n', 'f.' + btn);
      const open = el('button', 'btn', { 'data-open-folder': id, 'data-i18n': 'f.open' });
      row.append(name, path, second, open);
      host.append(row);
    });
  }

  function fillFolders() {
    const f = state.boot.folders || {};
    $$('[data-folder-path]').forEach((n) => { n.textContent = f[n.dataset.folderPath] || t('f.unknown'); });
    $$('[data-bind="folder-screenshots"]').forEach((n) => { n.textContent = f.screenshots || t('f.unknown'); });
  }

  function buildLanguageSelect(langs) {
    const sel = $('#lang-select');
    sel.textContent = '';
    langs.forEach((l) => { const o = el('option'); o.value = l.code; o.textContent = l.name; sel.append(o); });
  }

  // ---------- binding of settings controls ----------
  function syncControls() {
    const s = state.settings;
    $$('[data-bool]').forEach((b) => b.setAttribute('aria-checked', getPath(s, b.dataset.bool) ? 'true' : 'false'));
    $$('[data-select]').forEach((n) => {
      const v = getPath(s, n.dataset.select);
      const want = n.dataset.select === 'language' ? I18N.current() : String(v);
      if ($$('option', n).some((o) => o.value === want)) n.value = want;
    });
    $$('.theme').forEach((b) => b.setAttribute('aria-checked', b.dataset.themeId === s.theme ? 'true' : 'false'));
    const skipped = s.skipped_version;
    $('#skipped-text').textContent = skipped ? 'v' + skipped : t('set.skipped_d');
    $('#skipped-clear').disabled = !skipped;
  }

  async function selectTheme(id) {
    Themes.apply(id);
    await setSetting('theme', id);
    syncControls();
  }

  async function changeLanguage(code) {
    await setSetting('language', code);
    await I18N.load(I18N.resolve(code, window.__langs));
    I18N.apply(document);
    showPage(state.pageId);
    syncControls();
    if (window.Install) Install.render();
    if (window.Support) Support.render();
  }

  function bindStatic() {
    $$('.tab').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));
    $('#lang-chip').addEventListener('click', () => { showTab('settings'); showSub($('.page[data-page="settings"]'), 'general'); });
    $('#start-setup').addEventListener('click', () => { state.setupStarted = true; showTab('install'); if (window.Update) Update.setup(); });
    $$('.pills').forEach((nav) => {
      const page = nav.closest('.page');
      $$('.pill[data-sub]', nav).forEach((p) => p.addEventListener('click', () => showSub(page, p.dataset.sub)));
    });
    $$('[data-goto-sub]').forEach((b) => b.addEventListener('click', () => showSub(b.closest('.page'), b.dataset.gotoSub)));
    $$('[data-bool]').forEach((b) => b.addEventListener('click', async () => {
      await setSetting(b.dataset.bool, b.getAttribute('aria-checked') !== 'true');
      syncControls();
    }));
    $$('[data-select]').forEach((n) => n.addEventListener('change', async () => {
      const path = n.dataset.select;
      if (path === 'language') return changeLanguage(n.value);
      await setSetting(path, n.hasAttribute('data-number') ? Number(n.value) : n.value);
      syncControls();
    }));
    $('#skipped-clear').addEventListener('click', async () => { await setSetting('skipped_version', null); syncControls(); });
    $$('.check').forEach((c) => c.addEventListener('click', () => c.setAttribute('aria-checked', c.getAttribute('aria-checked') === 'true' ? 'false' : 'true')));

    // buttons whose feature is not built yet say so instead of doing nothing
    document.addEventListener('click', (e) => {
      const later = e.target.closest('[data-later]');
      if (later) { e.preventDefault(); toast(t('toast.later')); return; }
      const open = e.target.closest('[data-open-folder]');
      if (open) {
        Bridge.openFolder(open.dataset.openFolder).catch((err) => toast(err === 'missing' ? t('toast.folder_missing') : t('toast.folder_failed'), true));
      }
    });
    document.addEventListener('contextmenu', (e) => { if (!dev.has('dev')) e.preventDefault(); });

    // the design is 1280 x 760: the window may be any size, so the page is scaled to fit it (centred, proportions kept; GitHub #7)
    const fit = () => {
      let k = Math.min(innerWidth / 1280, innerHeight / 760);
      if (Math.abs(k - 1) < 0.004) k = 1;
      const x = Math.max(0, (innerWidth - 1280 * k) / 2), y = Math.max(0, (innerHeight - 760 * k) / 2);
      const app = $('#app');
      app.style.transformOrigin = '0 0';
      app.style.transform = k === 1 && x < 1 && y < 1 ? '' : `translate(${x.toFixed(2)}px, ${y.toFixed(2)}px) scale(${k.toFixed(4)})`;
      document.documentElement.style.setProperty('--ui-scale', k.toFixed(4));
    };
    window.addEventListener('resize', fit);
    fit();

    // window chrome: drag by the title bar and the top of the rail
    const dragStart = (e) => { if (e.button === 0 && !e.target.closest('button')) Bridge.drag(); };
    $('#head').addEventListener('mousedown', dragStart);
    $('.rail .logo').addEventListener('mousedown', dragStart);
    $('.rail .grow').addEventListener('mousedown', dragStart);
    $('#win-min').addEventListener('click', () => Bridge.minimise());
    $('#win-close').addEventListener('click', () => Bridge.close());
    window.addEventListener('error', (e) => Bridge.log('error', 'page error: ' + e.message));
    window.addEventListener('unhandledrejection', (e) => Bridge.log('error', 'page rejection: ' + e.reason));
  }

  function fillBindings() {
    $$('[data-bind="launcher-version"]').forEach((n) => { n.textContent = 'v' + state.boot.version; });
  }

  // ---------- self test (RUSTY_MCLA_SELFTEST): the page measures itself, Rust writes the report and exits ----------
  const TOKENS = ['bg', 'rail', 'panel', 'ink', 'muted', 'accent', 'accent2', 'onaccent', 'line', 'line2', 'wash', 'sky1', 'sky2', 'sky3', 'sky4', 'sun', 'bld', 'trail1', 'trail2', 'scrim', 'heroInk', 'heroAccent'];

  const rectOf = (n) => { const r = n.getBoundingClientRect(); return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)]; };

  async function selfTest() {
    const report = { ok: true, errors: [], version: state.boot.version, tauri: Bridge.available };
    const bad = (m) => { report.ok = false; report.errors.push(m); };
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    try {
      report.start = { theme: Themes.current(), language: I18N.current(), installed: state.installed, tab: state.tab, page: state.pageId };
      report.rail = rectOf($('.rail'));
      // 1. every page: card rectangles and anything that overflows its box
      report.pages = {};
      const savedTab = state.tab;
      for (const page of $$('.page')) {
        const id = page.dataset.page;
        $$('.page').forEach((p) => { p.hidden = p.dataset.page !== id; });
        const subs = $$('[data-sub-page]', page).map((s) => s.dataset.subPage);
        const names = subs.length ? subs : [''];
        for (const sub of names) {
          if (sub) $$('[data-sub-page]', page).forEach((s) => { s.hidden = s.dataset.subPage !== sub; });
          await wait(20);
          const key = id + (sub ? ':' + sub : '');
          const cards = $$('.card', page).filter((c) => c.offsetParent !== null).map(rectOf);
          const over = [];
          $$('.card, .btn, .field, .play', page).filter((n) => n.offsetParent !== null && !n.matches('.tbl-body')).forEach((n) => {
            if (n.scrollWidth > n.clientWidth + 1 && getComputedStyle(n).overflowX !== 'visible') over.push(['w', n.className, n.textContent.trim().slice(0, 30)]);
            if (n.scrollHeight > n.clientHeight + 1 && n.matches('.card, .btn, .play') && getComputedStyle(n).overflowY !== 'auto') over.push(['h', n.className, n.textContent.trim().slice(0, 30)]);
          });
          const outside = cards.filter((c) => c[0] < 128 || c[0] + c[2] > 1248 || c[1] < 88 || c[1] + c[3] > 728);
          report.pages[key] = { cards, overflow: over, outside, text: (page.innerText || '').length };
          if (outside.length) bad('card outside the content area on ' + key);
        }
      }
      state.tab = savedTab; refreshRail(); showPage(pageFor(savedTab));

      // 1b. the Install page's cards in every state of the build (free space under the last element, horizontal overflow), in three themes
      if (window.Install && Install.fitStates) {
        report.installFit = {};
        const prevTheme = Themes.current();
        for (const th of [prevTheme, 'alucard', 'dracula']) {
          Themes.apply(th);
          const f = Install.fitStates();
          report.installFit[th] = th === prevTheme ? f : { min: f.min, worst: f.worst, overflow: f.overflow };
          if (f.overflow.length) bad('install page overflows horizontally in ' + th + ': ' + f.overflow.join('; '));
          if (Object.values(f.min).some((v) => v < 0)) bad('install page: a card has no room under its last element in ' + th + ' ' + JSON.stringify(f.min));
        }
        Themes.apply(prevTheme);
      }

      // 2. all themes: computed token values
      report.themes = {};
      const prev = Themes.current();
      for (const th of Themes.list) {
        Themes.apply(th.id);
        const cs = getComputedStyle(document.documentElement);
        const row = {};
        TOKENS.forEach((k) => { row[k] = cs.getPropertyValue('--' + k).trim(); });
        row.attr = document.documentElement.getAttribute('data-theme');
        row.bodyBg = getComputedStyle(document.body).backgroundColor;
        report.themes[th.id] = row;
      }
      Themes.apply(prev);

      // 3. i18n: every key used exists, fallback works per key
      const used = new Set();
      $$('[data-i18n]').forEach((n) => used.add(n.getAttribute('data-i18n')));
      $$('[data-i18n-attr]').forEach((n) => n.getAttribute('data-i18n-attr').split(';').forEach((p) => used.add(p.slice(p.indexOf(':') + 1).trim())));
      const table = I18N.table();
      report.i18n = {
        used: used.size,
        missing: Array.from(used).filter((k) => !(k in table)),
        unused: Object.keys(table).filter((k) => !used.has(k) && !/^(act|pad|theme|mode|ctl\.(button|fixed\.|press_key|must|no_save|save_|saved|failed|ref_|kept|ask_text)|set\.mon_|dyn\.)/.test(k) && !k.startsWith('f.') && !k.startsWith('toast.')),
        placeholders_runtime: I18N.missing(),
      };
      const base = I18N.table();
      I18N.useStrings(base, { 'nav.home': 'ZZ' });
      report.i18n.fallback = [I18N.t('nav.home'), I18N.t('nav.install')];
      I18N.useStrings(base, {});

      // 4. settings persistence through Rust (theme and a switch), then read back
      const before = state.settings;
      await selectTheme('dracula');
      const toggled = !getPath(state.settings, 'remember_window');
      await setSetting('remember_window', toggled);
      const stored = await Bridge.boot();
      report.settings = { before: { theme: before.theme, remember_window: before.remember_window }, after: { theme: stored.settings.theme, remember_window: stored.settings.remember_window }, language: stored.settings.language, schema: stored.settings.schema };
      if (stored.settings.theme !== 'dracula' || stored.settings.remember_window !== toggled) bad('settings did not persist');
      await selectTheme(before.theme);
      await setSetting('remember_window', before.remember_window);

      // 5. tabs: locked tabs stay unreachable, first run shows the setup page
      const lockedTabs = {};
      LOCKED.forEach((k) => { showTab(k); lockedTabs[k] = state.tab === k; });
      report.lockedReachable = lockedTabs;
      report.window = { w: innerWidth, h: innerHeight, dpr: devicePixelRatio };
      if (!Bridge.available) bad('no tauri bridge');
    } catch (e) {
      bad('selftest threw: ' + (e && e.stack || e));
    }
    await Bridge.selftestReport(JSON.stringify(report, null, 1));
  }

  // ---------- start ----------
  async function init() {
    state.boot = await Bridge.boot();
    state.settings = state.boot.settings;
    state.installed = !!state.boot.installed || dev.get('installed') === '1';
    Themes.apply(state.settings.theme);

    const langs = await I18N.languages();
    window.__langs = langs;
    await I18N.load(I18N.resolve(state.settings.language, langs));

    buildKeyRows();
    buildThemeGrid();
    buildFolderRows();
    buildLanguageSelect(langs);
    I18N.apply(document);
    fillFolders();
    fillBindings();
    bindStatic();
    syncControls();
    const exportLogs = () => (window.Support ? Support.exportLogs() : Promise.resolve());
    if (window.Install) Install.init({ toast, setInstalled, exportLogs });
    if (window.Controls) Controls.init({ toast, showTab, showSub: (sub) => showSub($('.page[data-page="settings"]'), sub) });
    if (window.Update) Update.init({ toast, showTab, setSetting, syncControls, exportLogs, settings: () => state.settings, beginSetup: () => { state.setupStarted = true; showTab('install'); } });
    if (window.Support) Support.init({ toast, showTab, exportLogs, settings: () => state.settings });

    // preview pages (locked tabs): reachable only for design checks in a plain browser, never in the app
    const want = !Bridge.available && dev.get('page');
    const startTab = TABS.includes(state.settings.open_on) && tabAvailable(state.settings.open_on) ? state.settings.open_on : 'home';
    state.tab = want || startTab;
    refreshRail();
    if (want && $('.page[data-page="' + want + '"]')) { state.pageId = want; showPage(want); }
    else showPage(pageFor(state.tab));
    if (dev.get('sub')) { const p = $('.page[data-page="' + state.pageId + '"]'); if (p) showSub(p, dev.get('sub')); }
    Bridge.log('info', 'page ready: theme=' + Themes.current() + ' lang=' + I18N.current() + ' installed=' + state.installed);
    if (state.boot.install_test && window.Install) Install.test(state.boot.install_test, state.boot.install_test_cancel).then((r) => Bridge.selftestReport(JSON.stringify(r, null, 1)));
    else if (state.boot.controls_test && window.Controls) Controls.test().then((r) => Bridge.selftestReport(JSON.stringify(r, null, 1)));
    else if (state.boot.support_test && window.Support) Support.test(state.boot.support_test).then((r) => Bridge.selftestReport(JSON.stringify(r, null, 1)));
    else if (state.boot.update_test && window.Update) Update.test(state.boot.update_test).then((r) => Bridge.selftestReport(JSON.stringify(r, null, 1)));
    else if (state.boot.selftest) selfTest();
  }

  document.addEventListener('DOMContentLoaded', () => { init().catch((e) => { Bridge.log('error', 'init failed: ' + (e && e.stack || e)); document.body.textContent = String(e); }); });
})();
