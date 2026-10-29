// Support, save manager and Play on the page: Export Logs (Support tab, Home, every failure banner), the save manager (Support tab and the
// Home "Your save" card), the PLAY button with its states, the "closed unexpectedly" notice, and "go back to the previous game version".
// All work happens in Rust (src/support.rs over the mcla-support crate); this file draws results and asks for confirmation. Every sentence
// comes from the locales (`dyn.sup.*`, `dyn.play.*`). Nothing is sent anywhere: Export Logs writes a zip, the user attaches it by hand.
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const S = { app: null, save: null, play: null, crash: null, playErr: null, starting: false, exporting: false, exportDir: '', hw: null, timer: null, autoYes: false, asked: [] };
  const INC = ['launcher_logs', 'build_logs', 'game_logs', 'system', 'settings', 'save', 'screenshots'];

  // ---------- small helpers ----------
  const fmtWhen = (unix) => (unix ? new Intl.DateTimeFormat(I18N.current(), { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(unix * 1000)) : '');
  function fmtSize(b) {
    if (b < 1024) return b + ' B';
    if (b < 1048576) return (b / 1024).toFixed(1) + ' KB';
    return (b / 1048576).toFixed(1) + ' MB';
  }
  function errText(e, ns) {
    const key = 'dyn.' + (ns || 'sup') + '.err.' + ((e && e.code) || 'io');
    const txt = t(key);
    return txt === key ? (e && e.detail) || String(e) : txt;
  }
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };

  // ---------- confirmation dialog ----------
  function ask(title, text, ok) {
    S.asked.push({ title, text, ok });
    if (S.autoYes) return Promise.resolve(true);
    return new Promise((resolve) => {
      const back = $('#sup-dialog');
      $('#sup-dlg-title').textContent = title;
      $('#sup-dlg-text').textContent = text;
      const yes = $('#sup-dlg-yes'), no = $('#sup-dlg-no');
      yes.textContent = ok;
      const done = (v) => { back.hidden = true; yes.onclick = no.onclick = back.onclick = null; document.removeEventListener('keydown', key, true); resolve(v); };
      const key = (e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); done(false); } };
      document.addEventListener('keydown', key, true);
      yes.onclick = () => done(true);
      no.onclick = () => done(false);
      back.onclick = (e) => { if (e.target === back) done(false); };
      back.hidden = false;
      no.focus();
    });
  }

  // ---------- Export Logs ----------
  function includeFlags() {
    const o = {};
    INC.forEach((k) => { const c = $('.check[data-inc="' + k + '"]'); o[k] = c ? c.getAttribute('aria-checked') === 'true' : false; });
    return o;
  }

  async function refreshExport() {
    let info = null;
    try { info = await Bridge.exportInfo(); } catch (e) { Bridge.log('warn', 'export_info failed: ' + e); }
    S.exportDir = (info && info.dir) || '';
    const n = $('#sup-saved');
    if (n) n.textContent = S.exportDir ? t('dyn.sup.saved_to', [S.exportDir]) : t('dyn.sup.saved_to', ['...']);
    n && (n.title = S.exportDir);
  }

  async function exportLogs() {
    if (S.exporting) return;
    S.exporting = true;
    const b = $('#sup-export');
    const label = b && b.textContent;
    if (b) { b.disabled = true; b.textContent = t('dyn.sup.exporting'); }
    let result = null;
    try {
      result = await Bridge.exportLogs(includeFlags());
      const where = result.dir || S.exportDir;
      S.app.toast(t('dyn.sup.exported', [where]), false, { label: t('dyn.sup.show'), fn: () => Bridge.exportReveal().catch(() => S.app.toast(t('toast.folder_failed'), true)) });
      Bridge.log('info', 'export done: ' + result.path);
    } catch (e) {
      Bridge.log('error', 'export failed: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
    }
    S.exporting = false;
    if (b) { b.disabled = false; b.textContent = label; }
    return result;
  }

  async function changeExportDir() {
    let dir = null;
    try { dir = await Bridge.exportPickDir(); } catch (e) { Bridge.log('warn', 'folder dialog failed: ' + e); }
    if (!dir) return;
    await Bridge.exportSetDir(dir);
    await refreshExport();
  }

  async function openReport() {
    try {
      const url = await Bridge.reportUrl();
      await Bridge.openUrl(url);
      S.app.toast(t('dyn.sup.report_opened'));
    } catch (e) {
      Bridge.log('warn', 'report link failed: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
    }
  }

  // ---------- system card ----------
  async function loadSystem() {
    let s = null;
    try { s = await Bridge.systemSummary(); } catch (e) { Bridge.log('warn', 'system_summary failed: ' + e); }
    if (!s) return;
    S.hw = s;
    const set = (id, v) => { const n = $(id); if (n && v) n.textContent = v; };
    set('#sys-launcher', s.launcher);
    set('#sys-windows', s.windows);
    set('#sys-graphics', s.graphics);
    set('#sys-webview', s.webview);
    renderSystemGame();
  }
  function renderSystemGame() {
    const n = $('#sys-game');
    if (!n) return;
    const gv = S.play && S.play.game_version;
    n.textContent = gv ? 'v' + gv : t('dyn.sup.not_installed');
  }
  async function copySystem() {
    let text = '';
    try { text = await Bridge.systemText(); } catch (e) { S.app.toast(errText(e), true); return; }
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      const ta = document.createElement('textarea');
      ta.value = text; ta.style.position = 'fixed'; ta.style.opacity = '0';
      document.body.append(ta); ta.select();
      try { document.execCommand('copy'); } catch (e2) { /* nothing more to try */ }
      ta.remove();
    }
    S.app.toast(t('dyn.sup.copied'));
  }

  // ---------- save manager ----------
  async function loadSave() {
    try { S.save = await Bridge.saveView(); } catch (e) { Bridge.log('warn', 'save_view failed: ' + e); return; }
    renderSave();
  }

  const REASONS = ['before-update', 'manual', 'before-restore', 'before-import'];
  const reasonText = (r) => (REASONS.includes(r) ? t('dyn.sup.why_' + r.replace('before-', 'before_')) : r.toUpperCase());

  function renderSave() {
    const v = S.save;
    if (!v) return;
    const st = v.status;
    const cur = $('#sup-cur-d');
    if (cur) cur.textContent = st.exists ? t('dyn.sup.cur_d', [fmtWhen(st.modified), fmtSize(st.size)]) : t('dyn.sup.no_save');
    const chip = $('#sup-chip');
    if (chip) {
      chip.textContent = st.game_running ? t('dyn.sup.chip_running') : st.exists ? t('sup.in_use') : t('dyn.sup.chip_none');
      chip.classList.toggle('warn', !!st.game_running);
      chip.classList.toggle('accent', !st.game_running);
    }
    const keep = $('#sup-keep');
    if (keep) {
      if (!$$('option', keep).some((o) => Number(o.value) === st.keep)) { const o = el('option', '', String(st.keep)); o.value = String(st.keep); keep.append(o); }
      keep.value = String(st.keep);
    }
    const list = $('#sup-backups');
    if (list) {
      list.textContent = '';
      if (!v.backups.length) list.append(el('div', 'bk-empty', t('dyn.sup.no_backups')));
      v.backups.forEach((b) => {
        const row = el('div', 'bk');
        const when = el('span', '', fmtWhen(b.unix));
        const why = el('span', 'why', reasonText(b.reason));
        const sz = el('span', 'sz', fmtSize(b.size));
        const btn = el('button', 'btn sm', t('dyn.sup.restore'));
        btn.dataset.file = b.file;
        btn.addEventListener('click', () => restore(b));
        row.append(when, why, sz, btn);
        list.append(row);
      });
    }
    const home = $('#home-save-when');
    if (home) home.textContent = v.status.last_backup ? fmtWhen(v.status.last_backup.unix) : t('dyn.sup.home_none');
    const b1 = $('#sup-backup'), b2 = $('#home-backup');
    [b1, b2].forEach((b) => { if (b) b.disabled = !st.exists; });
    const imp = $('#sup-import');
    if (imp) imp.disabled = false;
  }

  async function backupNow() {
    try {
      S.save = await Bridge.saveBackupNow();
      renderSave();
      S.app.toast(t('dyn.sup.backed_up', [fmtWhen(S.save.status.last_backup && S.save.status.last_backup.unix)]));
    } catch (e) {
      Bridge.log('warn', 'backup failed: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
    }
  }

  async function withForce(run, title, okLabel) {
    try { return await run(false); } catch (e) {
      if (!e || e.code !== 'game_running') throw e;
      if (!(await ask(title, t('dyn.sup.running_text'), okLabel))) return null;
      return run(true);
    }
  }

  async function restore(b) {
    const ok = await ask(t('dyn.sup.restore_title'), t('dyn.sup.restore_text', [fmtWhen(b.unix), reasonText(b.reason)]), t('dyn.sup.restore_ok'));
    if (!ok) return;
    try {
      const v = await withForce((force) => Bridge.saveRestore(b.file, force), t('dyn.sup.running_title'), t('dyn.sup.restore_ok'));
      if (!v) return;
      S.save = v;
      renderSave();
      S.app.toast(t('dyn.sup.restored', [fmtWhen(b.unix)]));
    } catch (e) {
      Bridge.log('warn', 'restore failed: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
    }
  }

  // a port save's mod cars, each marked by whether its mod is installed (and on) here
  function modCarsText(info) {
    const cars = (info && info.mod_states) || [];
    if (!cars.length) return '';
    const key = { loaded: 'dyn.sup.imp_mod_loaded', off: 'dyn.sup.imp_mod_off', missing: 'dyn.sup.imp_mod_missing' };
    const lines = cars.map((c) => '  ' + t(key[c.state] || key.missing, [c.title]));
    const note = cars.some((c) => c.state !== 'loaded') ? '\n' + t('dyn.sup.imp_mods_note') : '';
    return '\n\n' + t('dyn.sup.imp_mods_head') + '\n' + lines.join('\n') + note;
  }

  async function importSave(pathArg) {
    let path = pathArg || null;
    if (!path) {
      try { path = await Bridge.saveImportPick(); } catch (e) { Bridge.log('warn', 'file dialog failed: ' + e); }
    }
    if (!path) return;
    // look at the file first: a refusal comes before the question, and a save of the original game gets a text that says what is converted
    let info;
    try {
      info = await Bridge.saveImportInspect(path);
    } catch (e) {
      Bridge.log('warn', 'import refused: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
      S.lastImportError = e;
      return;
    }
    const c = info && info.kind === 'original' ? info.converted : null;
    const modLines = modCarsText(info);
    const ok = c
      ? await ask(t('dyn.sup.import_orig_title'), t('dyn.sup.import_orig_text', [t('dyn.sup.platform_' + c.platform + (c.decrypted ? '_enc' : c.from_package ? '_pkg' : '')), c.cars, '$' + Number(c.money).toLocaleString('en-US'), c.reputation == null ? '-' : c.reputation, c.missions, c.missions_complete, c.statistics, c.collectables, c.editor_races, c.favourite_races]), t('dyn.sup.import_ok'))
      : await ask(t('dyn.sup.import_title'), t('dyn.sup.import_text') + modLines, t('dyn.sup.import_ok'));
    if (!ok) return;
    try {
      const v = await withForce((force) => Bridge.saveImport(path, force), t('dyn.sup.running_title'), t('dyn.sup.import_ok'));
      if (!v) return;
      S.save = v;
      renderSave();
      S.app.toast(v.converted ? t('dyn.sup.imported_orig', [v.converted.cars]) : t('dyn.sup.imported'));
      return v;
    } catch (e) {
      Bridge.log('warn', 'import refused: ' + JSON.stringify(e));
      S.app.toast(errText(e), true);
      S.lastImportError = e;
    }
  }

  // ---------- Play ----------
  const updateState = () => (window.Update && window.Update.state && window.Update.state.st) || null;

  function playState() {
    const info = S.play;
    if (!info) return { k: 'loading' };
    const up = updateState();
    if (info.updating || (up && up.busy === 'updating')) {
      const p = up && up.progress;
      const pct = p && p.total > 0 && ['download_game', 'download_launcher', 'extract_game'].includes(p.phase) ? Math.floor((p.done * 100) / p.total) : null;
      return { k: 'updating', pct };
    }
    if (!info.installed || !info.exe) return { k: 'setup' };
    if (S.starting) return { k: 'starting' };
    if (info.game_running) return { k: 'running' };
    return { k: 'ready' };
  }

  function renderPlay() {
    const b = $('#play');
    if (!b) return;
    const st = playState();
    const label = $('#play-label'), ic = $('#play-ic');
    const text = { loading: t('home.play'), ready: t('home.play'), setup: t('dyn.play.setup'), updating: st.pct != null ? t('dyn.play.updating_pct', [st.pct]) : t('dyn.play.updating'), starting: t('dyn.play.starting'), running: t('dyn.play.running') }[st.k];
    label.textContent = text;
    ic.style.display = st.k === 'ready' || st.k === 'loading' ? '' : 'none'; // (an <svg> has no `hidden` property)
    b.disabled = !['ready', 'loading'].includes(st.k);
    if (st.k === 'loading') b.disabled = true;
    b.dataset.state = st.k;
    const hs = $('#hero-state');
    if (hs) hs.textContent = { loading: t('home.ready'), ready: t('home.ready'), setup: t('dyn.play.state_setup'), updating: t('dyn.play.state_updating'), starting: t('dyn.play.state_starting'), running: t('dyn.play.state_running') }[st.k];
    // the same button on the Install page's end state
    const ip = $('#ins-play');
    if (ip) ip.disabled = !['ready'].includes(st.k);
    renderBanner();
    // the Settings row for the previous version
    const prev = $('#set-prev-btn'), prevd = $('#set-prev-d');
    if (prev && S.play) {
      prev.disabled = !S.play.previous || st.k === 'updating';
      prevd.textContent = S.play.previous ? t('dyn.play.prev_have', [S.play.previous_version || '?']) : t('set.prev_d');
    }
    renderSystemGame();
  }

  function renderBanner() {
    const el0 = $('#play-banner');
    if (!el0) return;
    const label = $('#pb-label'), text = $('#pb-text'), detail = $('#pb-detail'), actions = $('#pb-actions');
    const btn = (cls, key, fn) => { const b = el('button', cls, t(key)); b.addEventListener('click', fn); return b; };
    actions.textContent = '';
    detail.textContent = '';
    if (S.playErr) {
      const e = S.playErr;
      el0.className = 'ubanner pbanner bad';
      label.textContent = t('dyn.play.err_label');
      text.textContent = errText(e, 'play');
      detail.textContent = e.detail || '';
      detail.title = e.detail || '';
      if (e.code === 'missing_exe' || e.code === 'missing_data') actions.append(btn('btn primary', 'dyn.play.go_install', () => { S.playErr = null; S.app.showTab('install'); renderBanner(); }));
      actions.append(btn(e.code === 'missing_exe' || e.code === 'missing_data' ? 'btn edge' : 'btn primary', 'dyn.upd.export', () => exportLogs()), btn('link', 'dyn.upd.dismiss', () => { S.playErr = null; renderBanner(); }));
      el0.dataset.kind = 'error';
      el0.hidden = false;
    } else if (S.crash) {
      el0.className = 'ubanner pbanner info';
      label.textContent = t('dyn.play.crash_title');
      text.textContent = t('dyn.play.crash_text');
      if (S.crash.report) { detail.textContent = S.crash.report; detail.title = S.crash.report; }
      actions.append(btn('btn edge', 'dyn.upd.export', () => exportLogs()), btn('link', 'dyn.upd.dismiss', dismissCrash));
      el0.dataset.kind = 'crash';
      el0.hidden = false;
    } else {
      el0.hidden = true;
      el0.dataset.kind = '';
    }
  }

  async function dismissCrash() {
    S.crash = null;
    renderBanner();
    await Bridge.crashDismiss();
  }

  async function play() {
    if (S.starting) return null;
    const st = playState();
    if (st.k !== 'ready') return null;
    S.starting = true;
    S.playErr = null;
    S.crash = null;
    renderPlay();
    let r = null;
    try {
      r = await Bridge.play();
      Bridge.log('info', 'play ok: pid ' + r.pid + ', closing ' + r.closing);
    } catch (e) {
      Bridge.log('warn', 'play failed: ' + JSON.stringify(e));
      S.playErr = e;
    }
    S.starting = false;
    await refresh();
    return r;
  }

  async function goBack() {
    const info = S.play;
    if (!info || !info.previous) return;
    const ok = await ask(t('dyn.play.prev_title'), t('dyn.play.prev_text', [info.previous_version || '?', info.game_version || '?']), t('dyn.play.prev_ok'));
    if (!ok) return;
    try {
      S.play = await withForce((force) => Bridge.goBackPrevious(force), t('dyn.sup.running_title'), t('dyn.play.prev_ok'));
      if (!S.play) return;
      S.app.toast(t('dyn.play.prev_done', [S.play.game_version || '?']));
      if (window.Update) Update.poll();
      await loadSave();
      renderPlay();
    } catch (e) {
      Bridge.log('warn', 'go back failed: ' + JSON.stringify(e));
      S.app.toast(errText(e, 'play'), true);
    }
  }

  async function refresh() {
    try { S.play = await Bridge.playInfo(); } catch (e) { Bridge.log('warn', 'play_info failed: ' + e); }
    try { if (!S.playErr) S.crash = await Bridge.crashNotice(); } catch (e) { Bridge.log('warn', 'crash_notice failed: ' + e); }
    await loadSave();
    renderPlay();
  }

  function schedule() {
    clearTimeout(S.timer);
    const up = updateState();
    S.timer = setTimeout(async () => { if (!document.hidden) await refresh(); schedule(); }, up && up.busy === 'updating' ? 500 : 2000);
  }

  // ---------- start ----------
  function bind() {
    $$('.check[data-inc]').forEach((c) => c.addEventListener('click', () => {
      // the save holds progress and the screenshots are the player's own: ticking either says what it means
      if (c.dataset.inc === 'save' && c.getAttribute('aria-checked') === 'true') S.app.toast(t('dyn.sup.save_included'));
    }));
    $('#sup-export').addEventListener('click', exportLogs);
    $('#sup-change').addEventListener('click', (e) => { e.preventDefault(); changeExportDir(); });
    $('#sup-open-gh').addEventListener('click', openReport);
    ['#home-export', '#ins-export', '#ins-err-export'].forEach((id) => { const n = $(id); if (n) n.addEventListener('click', (e) => { e.preventDefault(); exportLogs(); }); });
    const hr = $('#home-report');
    if (hr) hr.addEventListener('click', openReport);
    $('#sup-copy').addEventListener('click', copySystem);
    $('#sup-backup').addEventListener('click', backupNow);
    $('#home-backup').addEventListener('click', backupNow);
    $('#sup-import').addEventListener('click', () => importSave());
    $('#sup-keep').addEventListener('change', async (e) => { try { S.save = await Bridge.saveSetKeep(Number(e.target.value)); renderSave(); } catch (err) { S.app.toast(errText(err), true); } });
    $('#play').addEventListener('click', play);
    const ip = $('#ins-play');
    if (ip) ip.addEventListener('click', play);
    const pb = $('#set-prev-btn');
    if (pb) pb.addEventListener('click', goBack);
  }

  function init(app) {
    S.app = app;
    bind();
    refreshExport();
    refresh().then(() => { loadSystem(); schedule(); });
    // when the language changes the page text is rebuilt from the locale: draw the dynamic parts again
    if (window.I18N && I18N.onChange) I18N.onChange(() => { renderSave(); renderPlay(); refreshExport(); });
  }

  // ---------- test hook (RUSTY_MCLA_SUPPORT_TEST) ----------
  // Steps (comma separated): snapshot, export[:save,shots], report, backup, restore:<n>, import:<path>, play, crash, dismiss, goback, keep:<n>, wait:<ms>.
  // Every confirmation is accepted (and recorded), so a run needs no clicks. Returns what the page showed.
  async function test(steps) {
    S.autoYes = true;
    const rep = { steps: [] };
    const shot = (tag, extra) => Object.assign({
      tag,
      play: { text: $('#play-label').textContent, disabled: $('#play').disabled, state: $('#play').dataset.state, hero: $('#hero-state').textContent },
      banner: $('#play-banner').hidden ? null : { kind: $('#play-banner').dataset.kind, label: $('#pb-label').textContent, text: $('#pb-text').textContent, detail: $('#pb-detail').textContent, buttons: $$('#pb-actions > *').map((x) => x.textContent) },
      saveCard: { cur: $('#sup-cur-d').textContent, chip: $('#sup-chip').textContent, keep: $('#sup-keep').value, home: $('#home-save-when').textContent, rows: $$('#sup-backups .bk').map((r) => r.innerText.replace(/\s+/g, ' ')) },
      system: { game: $('#sys-game').textContent, windows: $('#sys-windows').textContent, graphics: $('#sys-graphics').textContent, webview: $('#sys-webview').textContent, launcher: $('#sys-launcher').textContent },
      exportCard: { saved: $('#sup-saved').textContent, includes: includeFlags() },
      prevRow: { text: $('#set-prev-d').textContent, disabled: $('#set-prev-btn').disabled },
      toast: $('#toast').hidden ? null : $('#toast-text').textContent,
      asked: S.asked.splice(0),
    }, extra || {});
    await sleep(600);
    for (const step of steps.split(',').map((s) => s.trim()).filter(Boolean)) {
      const [name, arg] = step.split(/:(.*)/s);
      let extra = {};
      try {
        if (name === 'export') {
          const flags = (arg || '').split('+');
          INC.forEach((k) => { const c = $('.check[data-inc="' + k + '"]'); if (c) c.setAttribute('aria-checked', ['save', 'screenshots'].includes(k) ? String(flags.includes(k === 'screenshots' ? 'shots' : k)) : 'true'); });
          const r = await exportLogs();
          extra = { exported: r };
        } else if (name === 'report') {
          extra = { url: await Bridge.reportUrl() };
        } else if (name === 'backup') {
          $('#sup-backup').click(); await sleep(400);
        } else if (name === 'restore') {
          const rows = $$('#sup-backups .bk .btn');
          const b = rows[Number(arg || 0)];
          extra = { clicked: !!b };
          if (b) { b.click(); await sleep(600); }
        } else if (name === 'import') {
          const v = await importSave(arg);
          extra = { ok: !!v, error: S.lastImportError || null };
          S.lastImportError = null;
        } else if (name === 'play') {
          await refresh();
          extra = { before: playState().k };
          const r = await play();
          extra.result = r;
          extra.err = S.playErr;
          await sleep(300);
        } else if (name === 'playraw') {
          // asks Rust to start the game whatever the button says (the exe may have been deleted after the page drew)
          try { extra = { ok: await Bridge.play() }; } catch (e) { S.playErr = e; extra = { err: e }; }
          await sleep(300);
          await refresh();
        } else if (name === 'updcheck') {
          await Bridge.updateCheck(false);
          for (let i = 0; i < 100; i++) { await Update.poll(); const st = Update.state.st; if (st && st.busy === 'idle' && st.check.state !== 'not_checked') break; await sleep(100); }
        } else if (name === 'updstart') {
          await Bridge.updateNow();
          await sleep(300);
        } else if (name === 'sample') {
          const out = [];
          for (let i = 0; i < Number(arg || 10); i++) {
            await Update.poll(); await refresh();
            const up = updateState();
            out.push({ i, text: $('#play-label').textContent, disabled: $('#play').disabled, state: $('#play').dataset.state, busy: up && up.busy, phase: up && up.progress && up.progress.phase, done: up && up.progress && up.progress.done, total: up && up.progress && up.progress.total });
            await sleep(450);
          }
          extra = { samples: out };
        } else if (name === 'crash') {
          await refresh();
        } else if (name === 'dismiss') {
          const b = $$('#pb-actions button').find((x) => x.textContent === t('dyn.upd.dismiss'));
          if (b) b.click();
          await sleep(300);
          await refresh();
        } else if (name === 'goback') {
          await refresh();
          extra = { before: S.play };
          await goBack();
          await sleep(300);
        } else if (name === 'keep') {
          $('#sup-keep').value = arg; $('#sup-keep').dispatchEvent(new Event('change')); await sleep(300);
        } else if (name === 'wait') {
          await sleep(Number(arg) || 500);
        } else if (name === 'refresh') {
          await refresh();
        }
      } catch (e) {
        extra = { thrown: String((e && e.stack) || JSON.stringify(e)) };
      }
      rep.steps.push(shot(step, extra));
    }
    S.autoYes = false;
    return rep;
  }

  window.Support = { init, ask, exportLogs, play, openReport, render: () => { renderSave(); renderPlay(); refreshExport(); }, refresh, importSave, test, state: S };
})();
