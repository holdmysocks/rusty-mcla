// Update system on the page: the banner on Home (update available, downloading, stopped, rebuild needed, offline), the release notes
// card, the Updates card, Settings > General rows, the release list dialog and step 1 of the Install stepper (the game package).
// All work happens in Rust (src/update.rs over the mcla-updater crate); this file polls `update_status` and draws it. Every sentence comes
// from the locales (`dyn.upd.*`, `dyn.ver.*`, `dyn.notes.*`). Release notes arrive as HTML that Rust already sanitised (no scripts,
// no remote images, http(s) links only); links in them open in the system browser through the `open_url` command.
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);
  const dev = new URLSearchParams(location.search);

  const S = { st: null, app: null, timer: null, later: false, offlineDismissed: false, last: '', verRows: null };

  // ---------- helpers ----------
  function rel(unix) {
    if (!unix) return t('dyn.upd.never');
    const secs = Math.max(0, Math.round(Date.now() / 1000 - unix));
    const f = new Intl.RelativeTimeFormat(I18N.current(), { numeric: 'auto' });
    if (secs < 60) return f.format(0, 'second');
    if (secs < 3600) return f.format(-Math.round(secs / 60), 'minute');
    if (secs < 86400) return f.format(-Math.round(secs / 3600), 'hour');
    return f.format(-Math.round(secs / 86400), 'day');
  }
  const checkFailed = (st) => ['offline', 'failed', 'rate_limited'].includes(st.check.state);
  const checkedAgo = (st) => (st.check.checked_at ? t(checkFailed(st) ? 'dyn.upd.checked_failed' : 'dyn.upd.checked', [rel(st.check.checked_at)]) : t('dyn.upd.never'));
  function fmtDate(iso) {
    if (!iso) return '';
    const d = new Date(iso);
    return isNaN(d) ? '' : new Intl.DateTimeFormat(I18N.current(), { dateStyle: 'medium' }).format(d);
  }
  function clock(unix) {
    return new Intl.DateTimeFormat(I18N.current(), { timeStyle: 'short' }).format(new Date(unix * 1000));
  }
  function errText(e) {
    const key = 'dyn.upd.err.' + ((e && e.code) || 'io');
    const txt = t(key);
    return txt === key ? (e && e.detail) || String(e) : txt;
  }
  const busy = () => !!S.st && S.st.busy !== 'idle';
  // a check in the background (at most 15 s) must not make a button press vanish: wait for it first
  async function idle() { await poll(); for (let i = 0; i < 200 && busy(); i++) { await new Promise((r) => setTimeout(r, 100)); await poll(); } }
  const job = () => (S.st && S.st.progress && S.st.progress.job) || null;
  const gameVersion = () => (S.st && S.st.installed.game_version) || null;

  // ---------- which banner ----------
  const BANNER_JOBS = ['update', 'version', 'package'];

  function bannerKind(st) {
    if (st.busy === 'updating' && st.progress && BANNER_JOBS.includes(st.progress.job)) return 'working';
    if (st.error && BANNER_JOBS.includes(st.error.job)) return 'stopped';
    if (st.done && BANNER_JOBS.includes(st.done.job) && st.done.needs_rebuild) return 'rebuild';
    if (st.plan && st.plan.rebuild_pending) return S.later ? null : 'rebuild';
    if (st.done && BANNER_JOBS.includes(st.done.job)) return 'done';
    if (st.plan && st.plan.offer && !S.later) return 'available';
    const cs = st.check.state;
    if (['offline', 'rate_limited', 'failed'].includes(cs) && !S.offlineDismissed && st.check.enabled !== undefined) return 'offline';
    return null;
  }

  function phaseText(p) {
    const st = S.st;
    const ver = (st.release && st.release.version) || '';
    switch (p.phase) {
      case 'download_game': return t('dyn.upd.dl_game', [ver]);
      case 'download_launcher': return t('dyn.upd.dl_launcher', [ver]);
      case 'verify_game': return t('dyn.upd.verifying');
      case 'extract_game': return t('dyn.upd.extracting');
      case 'backup': return t('dyn.upd.backup');
      case 'swap_game': return t('dyn.upd.swapping');
      case 'verify_launcher': return t('dyn.upd.verifying_launcher');
      case 'apply_launcher': return t('dyn.upd.applying');
      default: return t('dyn.upd.working');
    }
  }

  function button(cls, key, fn) {
    const b = document.createElement('button');
    b.className = cls;
    b.textContent = t(key);
    b.addEventListener('click', fn);
    return b;
  }

  function retry() {
    const j = S.st && S.st.error && S.st.error.job;
    Bridge.updateClearResult().then(() => {
      if (j === 'setup') idle().then(() => Bridge.updateSetup(true).then(poll));
      else if (j === 'update') idle().then(() => Bridge.updateNow().then(poll));
      else if (j === 'package') pickAndUse();
      else Bridge.updateCheck(false);
      poll();
    });
  }

  function playAnyway() {
    S.later = true;
    S.offlineDismissed = true;
    Bridge.updateClearResult().then(poll);
  }

  function renderBanner() {
    const el = $('#upd-banner');
    if (!el) return;
    const st = S.st;
    const kind = st ? bannerKind(st) : null;
    if (!kind) { el.hidden = true; el.dataset.kind = ''; return; }
    const label = $('#ub-label'), text = $('#ub-text'), pct = $('#ub-pct'), bar = $('#ub-bar'), actions = $('#ub-actions');
    el.className = 'ubanner';
    label.textContent = '';
    text.textContent = '';
    pct.hidden = true;
    bar.hidden = true;
    actions.textContent = '';
    const line = (cls, s) => { const d = document.createElement('div'); d.className = cls; d.textContent = s; text.append(d); };
    const ver = (st.release && st.release.version) || '';
    const plan = st.plan || {};
    if (kind === 'available') {
      label.textContent = t('dyn.upd.label_avail');
      const settings = S.app.settings();
      const key = plan.launcher_first ? 'dyn.upd.avail_launcher_first' : settings.backup_before_update ? 'dyn.upd.avail' : 'dyn.upd.avail_nobackup';
      text.textContent = t(key, [plan.version || ver]);
      if (plan.update_needs_rebuild) line('ub-more', t('dyn.upd.rebuild_warn'));
      actions.append(
        button('btn primary', 'dyn.upd.now', async () => { S.later = false; await idle(); await Bridge.updateNow(); poll(); }),
        button('btn', 'dyn.upd.skip', skip),
        button('link', 'dyn.upd.later', () => { S.later = true; render(); }),
      );
    } else if (kind === 'working') {
      const p = st.progress;
      el.classList.add('working');
      text.textContent = phaseText(p);
      const known = p.total > 0 && ['download_game', 'download_launcher', 'extract_game'].includes(p.phase);
      if (known) {
        const v = Math.floor((p.done * 100) / p.total);
        pct.hidden = false; pct.textContent = v + '%';
        bar.hidden = false; bar.style.setProperty('--p', v);
      }
      if (['download_game', 'download_launcher', 'extract_game', 'verify_game', 'verify_launcher'].includes(p.phase)) actions.append(button('link', 'dyn.upd.cancel', () => Bridge.updateCancel()));
    } else if (kind === 'stopped') {
      el.classList.add('bad');
      label.textContent = t('dyn.upd.label_stopped');
      text.textContent = errText(st.error);
      if (st.error.detail) { line('ub-detail', st.error.detail); text.lastChild.title = st.error.detail; }
      actions.append(
        button('btn primary', 'dyn.upd.retry', retry),
        button('btn edge', 'dyn.upd.export', () => S.app.exportLogs()),
        button('link', 'dyn.upd.play_anyway', playAnyway),
      );
    } else if (kind === 'rebuild') {
      label.textContent = t('dyn.upd.label_rebuild');
      text.textContent = t('dyn.upd.rebuild');
      actions.append(
        button('btn primary', 'dyn.upd.rebuild_go', () => { Bridge.updateClearResult().then(poll); S.app.showTab('install'); }),
        button('link', 'dyn.upd.later', () => { S.later = true; Bridge.updateClearResult().then(poll); }),
      );
    } else if (kind === 'done') {
      label.textContent = t('dyn.upd.label_done');
      text.textContent = t('dyn.upd.done', [st.done.game_version || ver]);
      actions.append(button('link', 'dyn.upd.dismiss', () => Bridge.updateClearResult().then(poll)));
    } else if (kind === 'offline') {
      el.classList.add('info');
      label.textContent = t('dyn.upd.label_offline');
      const cs = st.check;
      if (cs.state === 'rate_limited') text.textContent = t('dyn.upd.rate', [cs.reset_at ? clock(cs.reset_at) : '...']);
      else if (cs.state === 'failed' && cs.code === 'manifest') text.textContent = t('dyn.upd.manifest_failed', [cs.detail || '']);
      else if (cs.state === 'failed') text.textContent = t('dyn.upd.failed');
      else text.textContent = t(st.installed.game_version ? 'dyn.upd.offline' : 'dyn.upd.offline_nogame');
      actions.append(
        button('btn', 'dyn.upd.retry', () => { S.offlineDismissed = false; Bridge.updateCheck(false).then(poll); }),
        button('link', 'dyn.upd.dismiss', () => { S.offlineDismissed = true; render(); }),
      );
    }
    el.dataset.kind = kind;
    el.hidden = false;
  }

  async function skip() {
    const v = S.st.plan && S.st.plan.version;
    if (!v) return;
    await S.app.setSetting('skipped_version', v);
    S.app.toast(t('dyn.upd.toast_skipped', [v]));
    S.app.syncControls();
    poll();
  }

  // ---------- cards and rows ----------
  function renderNotes() {
    const st = S.st;
    const r = st && st.release;
    const title = $('#notes-title'), ver = $('#notes-ver'), latest = $('#notes-latest'), date = $('#notes-date'), md = $('#notes-md'), full = $('#notes-full');
    if (!title) return;
    if (!r) {
      title.textContent = t('dyn.notes.none_title');
      ver.hidden = true; latest.hidden = true; date.textContent = '';
      md.innerHTML = '';
      const p = document.createElement('p');
      p.className = 'note';
      p.textContent = t('dyn.notes.none');
      md.append(p);
      full.hidden = true;
      return;
    }
    title.textContent = r.name;
    ver.hidden = false; ver.textContent = 'v' + r.version;
    latest.hidden = false;
    date.textContent = fmtDate(r.published_at);
    if (md.dataset.tag !== r.tag + ':' + r.notes_html.length) { md.innerHTML = r.notes_html; md.dataset.tag = r.tag + ':' + r.notes_html.length; }
    full.hidden = false;
    full.dataset.href = r.html_url;
  }

  function cardState(st) {
    const cs = st.check, plan = st.plan;
    if (st.busy === 'checking') return { text: t('dyn.upd.card_checking'), warn: false };
    if (st.busy === 'updating') return { text: t('dyn.upd.card_updating'), warn: false };
    if (plan && plan.rebuild_pending) return { text: t('dyn.upd.card_rebuild'), warn: true };
    if (plan && plan.offer) return { text: t('dyn.upd.card_available', [plan.version]), warn: false };
    if (plan && plan.skipped && (plan.launcher_update || plan.game_update)) return { text: t('dyn.upd.card_skipped', [plan.version]), warn: false };
    if (cs.state === 'rate_limited') return { text: t('dyn.upd.card_rate', [cs.reset_at ? clock(cs.reset_at) : '...']), warn: true };
    if (['offline', 'failed'].includes(cs.state)) return { text: t('dyn.upd.card_failed'), warn: true };
    if (!cs.checked_at) return { text: t('dyn.upd.never'), warn: false };
    return { text: t('dyn.upd.card_latest'), warn: false };
  }

  function renderCard() {
    const st = S.st;
    if (!st || !$('#upd-card-state')) return;
    const c = cardState(st);
    $('#upd-card-state').textContent = c.text;
    $('#upd-card-icon').classList.toggle('warn', c.warn);
    $('#upd-card-icon').innerHTML = '<use href="#' + (c.warn ? 'i-warn' : 'i-check') + '"/>';
    $('#upd-card-checked').textContent = checkedAgo(st);
    const b = $('#upd-card-check');
    b.disabled = busy();
    // head status
    let head;
    if (st.busy === 'checking') head = t('dyn.upd.card_checking');
    else if (st.plan && st.plan.offer) head = t('dyn.upd.head_available', [st.plan.version]);
    else if (['offline', 'failed', 'rate_limited'].includes(st.check.state)) head = t('dyn.upd.head_failed');
    else if (st.check.checked_at) head = t('dyn.upd.head_checked', [rel(st.check.checked_at)]);
    else head = st.check.enabled ? t('dyn.upd.never') : t('dyn.upd.head_off');
    $('#head-status').textContent = head;
  }

  function renderSettings() {
    const st = S.st;
    if (!st) return;
    const gv = st.installed.game_version;
    const d = $('#set-installed-d');
    if (d) d.textContent = t('dyn.upd.set_line', [st.installed.launcher, gv ? t('dyn.upd.game_ver', [gv]) : t('dyn.upd.no_game'), checkedAgo(st)]);
    const cn = $('#set-check-now'), ab = $('#about-check');
    if (cn) cn.disabled = busy();
    if (ab) ab.disabled = busy();
    $$('[data-i18n="sys.v_ver"]').forEach((n) => { n.textContent = gv ? t('dyn.upd.game_ver', [gv]) : t('dyn.upd.no_game'); });
    const rl = $('#hero-line');
    if (rl) rl.textContent = gv ? t('home.ready_line', [gv]) : t(unreached(st) ? 'dyn.upd.ready_no_game_offline' : 'dyn.upd.ready_no_game');
    $$('[data-web]').forEach((a) => {
      // "licences" is a file in the repository (THIRD-PARTY-NOTICES.md), not a field of the release links
      a.dataset.href = st.links[a.dataset.web] || (a.dataset.web === 'licences' && st.links.repo ? st.links.repo + '/blob/main/THIRD-PARTY-NOTICES.md' : '');
    });
  }

  // ---------- step 1 of the Install stepper: the game package ----------
  function step1() {
    const st = S.st;
    const none = { s: 'todo', text: t('dyn.upd.st1_wait') };
    if (!st) return none;
    const pk = ['setup', 'package'];
    if (st.busy === 'updating' && st.progress && pk.includes(st.progress.job)) {
      const p = st.progress;
      let text = phaseText(p);
      if (p.total > 0 && p.phase === 'download_game') text = t('dyn.upd.st1_run', [Math.floor((p.done * 100) / p.total)]);
      return { s: 'now', text };
    }
    if (st.busy === 'updating' && st.progress && st.progress.job === 'setup') return { s: 'now', text: t('dyn.upd.st1_check') };
    if (st.error && pk.includes(st.error.job)) return { s: 'now', text: st.error.code === 'offline' ? t('dyn.upd.st1_noreach') : t('dyn.upd.st1_error'), actions: true, title: errText(st.error) + (st.error.detail ? ' (' + st.error.detail + ')' : '') };
    const gv = st.installed.game_version;
    if (gv) return { s: 'done', text: t('dyn.upd.st1_done', [gv]) };
    if (unreached(st)) return { s: 'now', text: t('dyn.upd.st1_noreach'), actions: true };
    return Object.assign(none, { s: 'now' });
  }

  // no game files and the last look for a release found no usable answer from GitHub (no connection, a limit or an HTTP error)
  function unreached(st) {
    return !st.installed.game_version && ['offline', 'rate_limited', 'failed'].includes(st.check.state) && st.busy === 'idle';
  }

  function renderStepOne() {
    const v = step1();
    const a = $('#pkg-actions');
    if (a) a.hidden = !(v.actions || (v.s === 'now' && S.st && !busy() && !S.st.installed.game_version));
    const small = $('[data-step-d="1"]');
    if (small) small.title = v.title || '';
    if (window.Install) Install.render();
  }

  // ---------- polling ----------
  function render() {
    renderBanner(); renderNotes(); renderCard(); renderSettings(); renderStepOne();
  }

  function mock(kind) {
    const base = {
      busy: 'idle', check: { state: 'fresh', checked_at: Math.floor(Date.now() / 1000) - 240, enabled: true },
      release: { tag: 'v0.2.0', version: '0.2.0', name: 'Rusty MCLA 0.2.0', published_at: '2026-10-15T10:00:00Z', html_url: 'https://github.com/OWNER/rusty-mcla/releases/tag/v0.2.0', notes_html: '<h2>Added</h2><ul><li>Release notes on the Home page, rendered from the GitHub release body.</li><li>Update prompts you can always skip.</li></ul><h2>Fixed</h2><ul><li>The launcher starts with no network.</li></ul><h2>Known issues</h2><ul><li>Export Logs is not built yet.</li></ul>' },
      plan: { version: '0.2.0', launcher_update: true, game_update: true, first_install: false, launcher_first: false, update_needs_rebuild: false, rebuild_pending: false, skipped: false, offer: true },
      installed: { launcher: '0.1.0', game_version: '0.1.0', game_data_version: 1, built_data_version: 1 },
      progress: null, error: null, done: null, skipped_version: null, game_bytes: 40000000, launcher_bytes: 5000000, key_set: true,
      links: { repo: 'https://github.com/OWNER/rusty-mcla', releases: 'https://github.com/OWNER/rusty-mcla/releases', issues: 'https://github.com/OWNER/rusty-mcla/issues/new' },
    };
    if (kind === 'downloading') { base.busy = 'updating'; base.progress = { job: 'update', phase: 'download_game', done: 15200000, total: 40000000 }; }
    if (kind === 'stopped') base.error = { code: 'hash', detail: 'sha256 is 3f2a..., the manifest says 9c1d...', job: 'update' };
    if (kind === 'offline') { base.check = { state: 'offline', checked_at: Math.floor(Date.now() / 1000) - 7200, enabled: true }; base.plan.offer = false; }
    if (kind === 'rate') { base.check = { state: 'rate_limited', reset_at: Math.floor(Date.now() / 1000) + 1500, checked_at: Math.floor(Date.now() / 1000) - 7200, enabled: true }; base.plan.offer = false; }
    if (kind === 'nogame') { base.check = { state: 'offline', checked_at: 0, enabled: true }; base.plan.offer = false; base.installed.game_version = null; base.release = null; }
    if (kind === 'rebuild') { base.plan.offer = false; base.plan.rebuild_pending = true; base.installed.game_data_version = 2; }
    if (kind === 'rebuildwarn') base.plan.update_needs_rebuild = true;
    if (kind === 'done') base.done = { job: 'update', game_version: '0.2.0', needs_rebuild: false };
    if (kind === 'latest') { base.plan.offer = false; base.plan.launcher_update = false; base.plan.game_update = false; base.installed.launcher = '0.2.0'; base.installed.game_version = '0.2.0'; }
    return base;
  }

  async function poll() {
    let st = null;
    try { st = Bridge.available ? await Bridge.updateStatus() : (dev.get('upd') ? mock(dev.get('upd')) : null); } catch (e) { Bridge.log('error', 'update_status failed: ' + e); }
    if (!st) return;
    const key = JSON.stringify(st);
    const first = !S.st;
    S.st = st;
    // an error or result the user already handled in the page and Rust cleared stays cleared; a new job re-arms the banner
    if (st.busy === 'updating') { S.later = false; }
    if (st.error && ['setup', 'package'].includes(st.error.job)) {
      // these jobs have no banner on the page the user is on (first run / Install), so say it once as a toast too
      const k = st.error.code + '|' + st.error.detail;
      if (S.toasted !== k) { S.toasted = k; S.app.toast(errText(st.error), true); }
    } else S.toasted = null;
    if (key !== S.last || first) { S.last = key; render(); }
    else renderCard(); // "checked 5 min ago" ages on its own
    schedule();
  }

  function schedule() {
    clearTimeout(S.timer);
    S.timer = setTimeout(poll, busy() ? 250 : 2000);
  }

  // ---------- actions ----------
  async function checkNow(fromButton) {
    if (busy()) return;
    S.offlineDismissed = false;
    if (fromButton && S.st && S.st.check.enabled === false) S.app.toast(t('dyn.upd.toast_off'));
    await Bridge.updateCheck(false);
    await poll();
    // the check runs in the background; tell the user what came out of a manual one
    for (let i = 0; i < 160 && S.st.busy === 'checking'; i++) { await new Promise((r) => setTimeout(r, 100)); await poll(); }
    const st = S.st;
    if (!fromButton) return;
    if (st.plan && st.plan.offer) S.app.toast(t('dyn.upd.toast_available', [st.plan.version]));
    else if (['offline', 'failed', 'rate_limited'].includes(st.check.state)) S.app.toast(t('dyn.upd.toast_failed'), true);
    else S.app.toast(t('dyn.upd.toast_latest'));
  }

  async function pickAndUse() {
    let path = null;
    try { path = await Bridge.pickPackage(); } catch (e) { Bridge.log('error', 'file dialog failed: ' + e); }
    if (!path) return;
    S.later = false;
    await Bridge.updateClearResult();
    await idle();
    await Bridge.updateUsePackage(path);
    await poll();
  }

  async function useLocalPackage() { await pickAndUse(); }

  async function setup() {
    if (S.st && S.st.installed.game_version && !(S.st.error)) return;
    await Bridge.updateClearResult();
    await idle();
    await Bridge.updateSetup(true);
    await poll();
  }

  // ---------- the release list dialog ----------
  function closeDialog() { $('#ver-dialog').hidden = true; }

  async function openDialog() {
    const dlg = $('#ver-dialog'), list = $('#ver-list'), status = $('#ver-status');
    dlg.hidden = false;
    list.textContent = '';
    const loading = document.createElement('div');
    loading.className = 'ver-empty';
    loading.textContent = t('dyn.ver.loading');
    list.append(loading);
    status.textContent = '';
    let rows;
    try { rows = await Bridge.updateList(); } catch (e) {
      list.textContent = '';
      const d = document.createElement('div');
      d.className = 'ver-empty';
      d.textContent = t('dyn.ver.failed', [errText(e)]);
      list.append(d);
      return;
    }
    S.verRows = rows;
    list.textContent = '';
    if (!rows.length) { const d = document.createElement('div'); d.className = 'ver-empty'; d.textContent = t('dyn.ver.empty'); list.append(d); return; }
    status.textContent = t('dyn.ver.count', [rows.length]);
    rows.forEach((r) => {
      const row = document.createElement('div');
      row.className = 'ver-row';
      const v = document.createElement('span'); v.className = 'v'; v.textContent = 'v' + r.version;
      const n = document.createElement('span'); n.className = 'n'; n.textContent = r.name; n.title = r.name;
      const d = document.createElement('span'); d.className = 'd'; d.textContent = fmtDate(r.published_at);
      row.append(v, n, d);
      if (r.installed) { const c = document.createElement('span'); c.className = 'chip accent'; c.textContent = t('dyn.ver.current'); row.append(c); }
      else {
        const b = document.createElement('button'); b.className = 'btn sm'; b.textContent = t('dyn.ver.install'); b.dataset.tag = r.tag;
        b.addEventListener('click', async () => { closeDialog(); S.later = false; await Bridge.updateClearResult(); await idle(); await Bridge.updateInstallVersion(r.tag); S.app.showTab('home'); await poll(); });
        row.append(b);
      }
      list.append(row);
    });
  }

  // ---------- start ----------
  function init(app) {
    S.app = app;
    $('#upd-card-check').addEventListener('click', () => checkNow(true));
    $('#set-check-now').addEventListener('click', () => checkNow(true));
    $('#about-check').addEventListener('click', () => checkNow(true));
    $('#set-choose-ver').addEventListener('click', openDialog);
    $('#ver-close').addEventListener('click', closeDialog);
    $('#ver-dialog').addEventListener('click', (e) => { if (e.target.id === 'ver-dialog') closeDialog(); });
    $('#fr-local-package').addEventListener('click', (e) => { e.preventDefault(); app.beginSetup(); useLocalPackage(); });
    $('#pkg-local').addEventListener('click', (e) => { e.preventDefault(); useLocalPackage(); });
    $('#pkg-retry').addEventListener('click', (e) => { e.preventDefault(); retry(); });
    const hd = $('#pkg-dialog');
    const hide = () => { hd.hidden = true; };
    $('#pkg-howto').addEventListener('click', (e) => { e.preventDefault(); hd.hidden = false; });
    $('#pkg-dlg-close').addEventListener('click', hide);
    $('#pkg-dlg-web').addEventListener('click', () => { const u = S.st && S.st.links && S.st.links.releases; if (u) Bridge.openUrl(u); });
    $('#pkg-dlg-pick').addEventListener('click', () => { hide(); useLocalPackage(); });
    // links in the release notes and the web links open in the system browser (Rust opens http(s) only)
    const open = (e) => {
      const a = e.target.closest('a[href], [data-href]');
      if (!a) return;
      const href = a.dataset.href || a.getAttribute('href');
      if (!href || href === '#') return;
      e.preventDefault();
      Bridge.openUrl(href).catch((err) => { Bridge.log('warn', 'link refused: ' + err); app.toast(t('toast.folder_failed'), true); });
    };
    $('#notes-md').addEventListener('click', open);
    $('#notes-full').addEventListener('click', open);
    $$('[data-web]').forEach((a) => a.addEventListener('click', open));
    poll().then(() => {
      // check on open, if the user wants that (Rust also refuses when the setting is off)
      if (Bridge.available && app.settings().check_updates) Bridge.updateCheck(true).then(poll);
    });
  }

  // ---------- test hook (RUSTY_MCLA_UPDATE_TEST) ----------
  // Steps (comma separated): check, skip, clear, update, setup, list, local:<path>, wait. Drives the page like a user would and
  // returns what it showed. The launcher writes the result into the self-test report.
  async function test(steps) {
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const rep = { steps: [], samples: [] };
    const snap = (tag) => {
      const st = S.st || {};
      const b = $('#upd-banner');
      return {
        tag,
        banner: b.hidden ? null : { kind: b.dataset.kind, label: $('#ub-label').textContent, text: $('#ub-text').textContent, pct: $('#ub-pct').hidden ? null : $('#ub-pct').textContent, bar: $('#ub-bar').hidden ? null : $('#ub-bar').style.getPropertyValue('--p'), buttons: $$('#ub-actions > *').map((x) => x.textContent) },
        notes: { title: $('#notes-title').textContent, ver: $('#notes-ver').hidden ? null : $('#notes-ver').textContent, latest: !$('#notes-latest').hidden, date: $('#notes-date').textContent, html: $('#notes-md').innerHTML, text: $('#notes-md').innerText, fullLink: $('#notes-full').hidden ? null : $('#notes-full').dataset.href },
        card: { state: $('#upd-card-state').textContent, checked: $('#upd-card-checked').textContent, head: $('#head-status').textContent, button: $('#upd-card-check').textContent },
        settings: { installed: $('#set-installed-d').textContent, skipped: $('#skipped-text').textContent, clearDisabled: $('#skipped-clear').disabled },
        hero: $('#hero-line').textContent,
        step1: { text: $('[data-step-d="1"]').textContent, cls: $('.stepper .st[data-step="1"]').className, actions: !$('#pkg-actions').hidden },
        status: { busy: st.busy, check: st.check && st.check.state, plan: st.plan, installed: st.installed, error: st.error, done: st.done, skipped: st.skipped_version },
      };
    };
    const waitFor = async (fn, ms) => { for (let i = 0; i < ms / 100; i++) { await poll(); if (fn()) return true; await sleep(100); } return false; };
    for (const step of steps.split(',').map((s) => s.trim()).filter(Boolean)) {
      const [name, arg] = step.split(/:(.*)/s);
      if (name === 'check') {
        await waitFor(() => S.st && S.st.busy === 'idle' && S.st.check.state !== 'not_checked', 20000);
        rep.steps.push(snap('check'));
      } else if (name === 'recheck') {
        $('#upd-card-check').click();
        await sleep(300);
        await waitFor(() => S.st.busy === 'idle', 20000);
        rep.steps.push(snap('recheck'));
      } else if (name === 'skip') {
        const b = $$('#ub-actions button').find((x) => x.textContent === t('dyn.upd.skip'));
        if (b) b.click();
        await sleep(600);
        await waitFor(() => !S.st.plan || !S.st.plan.offer, 5000);
        rep.steps.push(snap('skip'));
      } else if (name === 'clear') {
        $('#skipped-clear').click();
        await sleep(600);
        await waitFor(() => S.st.plan && S.st.plan.offer, 5000);
        rep.steps.push(snap('clear'));
      } else if (name === 'update') {
        const b = $$('#ub-actions button').find((x) => x.textContent === t('dyn.upd.now'));
        if (!b) { rep.steps.push(Object.assign(snap('update'), { error: 'no UPDATE NOW button' })); continue; }
        const t0 = Date.now();
        b.click();
        let applied = false;
        for (let i = 0; i < 900; i++) {
          await sleep(100);
          await poll();
          const st = S.st;
          if (st.busy === 'updating') { const s = snap('working'); s.at = Date.now() - t0; if (rep.samples.length < 400) rep.samples.push({ at: s.at, kind: s.banner && s.banner.kind, text: s.banner && s.banner.text, pct: s.banner && s.banner.pct, phase: st.progress && st.progress.phase, done: st.progress && st.progress.done, total: st.progress && st.progress.total }); }
          if (st.progress && st.progress.phase === 'apply_launcher' && st.progress.done === 1) { applied = true; break; }
          if (st.busy === 'idle' && i > 3) break;
        }
        const s = snap('after update'); s.ms = Date.now() - t0; s.applied = applied;
        rep.steps.push(s);
      } else if (name === 'setup') {
        $('#start-setup') && $('#start-setup').click();
        await sleep(500);
        await waitFor(() => S.st.busy === 'idle', 60000);
        rep.steps.push(snap('setup'));
      } else if (name === 'list') {
        await openDialog();
        rep.steps.push({ tag: 'list', rows: $$('#ver-list .ver-row').map((r) => r.innerText.replace(/\s+/g, ' ')), status: $('#ver-status').textContent });
        const [mode, want] = (arg || '').split(':');
        const b = want ? $('#ver-list .btn[data-tag="' + want + '"]') : $('#ver-list .btn');
        if (mode === 'install' && b) { b.click(); await sleep(500); await waitFor(() => S.st.busy === 'idle', 60000); rep.steps.push(snap('version installed')); }
        else closeDialog();
      } else if (name === 'local') {
        await Bridge.updateClearResult();
        await idle();
        await Bridge.updateUsePackage(arg);
        await sleep(400);
        await waitFor(() => S.st.busy === 'idle', 60000);
        rep.steps.push(snap('local package'));
      } else if (name === 'wait') {
        await sleep(Number(arg) || 500);
      }
    }
    return rep;
  }

  window.Update = { init, setup, step1, render, test, poll, state: S, useLocalPackage };
})();
