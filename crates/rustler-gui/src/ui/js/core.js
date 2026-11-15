// Rustler GUI core: helpers, API client, jobs, router. Plain JS, no build step.
// Every action is a registry command via POST /api/run (or /api/jobs for background runs).
(function () {
  'use strict';
  var R = window.R = window.R || {};

  var T = new URLSearchParams(location.search).get('t') || '';
  var S = R.S = {
    cmds: {},            // name -> spec (feature detection)
    project: '',         // project dir typed by the user ('' = server default)
    projectInfo: null,
    view: 'project',
    arg: '',
    cleanup: [],         // functions run when leaving a view
    busy: 0,
    sel: { source_id: '', start_unit: '', end_unit: '' }
  };
  R.T = T;
  R.FALLBACK_STYLES = ['bold-karaoke', 'clean-white', 'boxed-highlight', 'hormozi-pop'];

  // ------------------------------------------------------------ dom
  function h(tag, attrs) {
    var e = document.createElement(tag);
    var a = attrs || {};
    Object.keys(a).forEach(function (k) {
      var v = a[k];
      if (v === null || v === undefined || v === false) return;
      if (k === 'class') e.className = v;
      else if (k === 'style' && typeof v === 'object') Object.assign(e.style, v);
      else if (k.indexOf('on') === 0 && typeof v === 'function') e.addEventListener(k.slice(2), v);
      else if (k === 'value') e.value = v;
      else if (k === 'checked' || k === 'disabled' || k === 'hidden' || k === 'open' || k === 'selected') e[k] = true;
      else e.setAttribute(k, v === true ? '' : String(v));
    });
    for (var i = 2; i < arguments.length; i++) add(e, arguments[i]);
    return e;
  }
  function add(e, c) {
    if (c === null || c === undefined || c === false) return;
    if (Array.isArray(c)) { c.forEach(function (x) { add(e, x); }); return; }
    e.appendChild(c.nodeType ? c : document.createTextNode(String(c)));
  }
  function clear(e) { while (e.firstChild) e.removeChild(e.firstChild); return e; }
  function $(id) { return document.getElementById(id); }
  R.h = h; R.add = add; R.clear = clear; R.$ = $;

  // ------------------------------------------------------------ features
  R.has = function (name) { return !!S.cmds[name]; };
  R.hasAny = function (names) { return names.some(R.has); };
  R.missing = function (names) { return names.filter(function (n) { return !R.has(n); }); };
  R.spec = function (name) { return S.cmds[name] || null; };
  R.props = function (name) { var s = S.cmds[name]; return (s && s.input_schema && s.input_schema.properties) || {}; };
  R.hasProp = function (name, prop) { return Object.prototype.hasOwnProperty.call(R.props(name), prop); };
  R.cmdsWithPrefix = function (prefix) { return Object.keys(S.cmds).filter(function (n) { return n.indexOf(prefix) === 0; }).sort(); };
  R.na = function () {
    var names = Array.prototype.slice.call(arguments);
    return h('div', { class: 'na' }, 'Not available in this build: ', names.join(', '),
      '. It appears here automatically once the command is registered.');
  };

  // ------------------------------------------------------------ format
  R.fileUrl = function (p) { return '/api/file?path=' + encodeURIComponent(p) + '&t=' + encodeURIComponent(T) + (S.project ? '&project=' + encodeURIComponent(S.project) : ''); };
  // For files that may legitimately not exist yet (204 instead of 404).
  R.optUrl = function (p) { return R.fileUrl(p) + '&optional=1'; };
  R.frameUrl = function (sid, t, w) {
    return '/api/frame?source_id=' + encodeURIComponent(sid) + '&at=' + (Math.round(Math.max(0, t) * 1000) / 1000) + '&w=' + (w || 640) +
      '&t=' + encodeURIComponent(T) + (S.project ? '&project=' + encodeURIComponent(S.project) : '');
  };
  R.fmt = function (t) {
    t = Number(t);
    if (!isFinite(t)) return '-';
    var neg = t < 0; t = Math.abs(t);
    var m = Math.floor(t / 60), s = t - m * 60;
    return (neg ? '-' : '') + m + ':' + (s < 10 ? '0' : '') + s.toFixed(1);
  };
  // mm:ss.ff (ff = frame number within the second)
  R.tc = function (t, fps) {
    t = Math.max(0, Number(t) || 0); fps = fps || 30;
    var total = Math.round(t * fps);
    var f = total % Math.round(fps);
    var secs = Math.floor(total / Math.round(fps));
    var m = Math.floor(secs / 60), s = secs % 60;
    return (m < 10 ? '0' : '') + m + ':' + (s < 10 ? '0' : '') + s + '.' + (f < 10 ? '0' : '') + f;
  };
  R.short = function (v, n) {
    var s = typeof v === 'string' ? v : JSON.stringify(v);
    if (s === undefined) s = '';
    return s.length > (n || 80) ? s.slice(0, n || 80) + '...' : s;
  };
  R.pretty = function (v) { try { return JSON.stringify(v, null, 2); } catch (e) { return String(v); } };
  R.listOf = function (d) {
    var keys = Array.prototype.slice.call(arguments, 1);
    if (Array.isArray(d)) return d;
    if (d && typeof d === 'object') {
      for (var i = 0; i < keys.length; i++) if (Array.isArray(d[keys[i]])) return d[keys[i]];
      var vals = Object.keys(d).map(function (k) { return d[k]; });
      for (var j = 0; j < vals.length; j++) {
        var v = vals[j];
        if (Array.isArray(v) && (v.length === 0 || typeof v[0] === 'object')) return v;
      }
    }
    return [];
  };
  R.store = function (k, v) { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch (e) { /* storage off */ } return null; };
  R.clamp = function (v, a, b) { return Math.max(a, Math.min(b, v)); };
  R.round = function (v, n) { var p = Math.pow(10, n === undefined ? 4 : n); return Math.round(v * p) / p; };

  // ------------------------------------------------------------ feedback
  R.toast = function (msg, kind) {
    var el = h('div', { class: 'toast ' + (kind || '') }, msg);
    $('toasts').appendChild(el);
    setTimeout(function () { if (el.parentNode && !el.classList.contains('sticky')) el.parentNode.removeChild(el); }, kind === 'bad' ? 9000 : 4000);
    return el;
  };
  function busy(d) {
    S.busy = Math.max(0, S.busy + d);
    var b = $('busy'); if (b) b.hidden = S.busy === 0;
  }

  // ------------------------------------------------------------ api
  function post(url, body) {
    return fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-rustler-token': T },
      body: JSON.stringify(body)
    }).then(function (r) {
      if (r.status === 401) { R.sessionLost(); return { ok: false, error: { message: 'session ended (server restarted)', code: 'usage' } }; }
      return r.json();
    }).catch(function (e) { return { ok: false, error: { message: String(e), code: 'internal' } }; });
  }
  R.call = function (command, input) {
    busy(1);
    var body = { command: command, input: input || {} };
    if (S.project) body.project = S.project;
    return post('/api/run', body).then(function (env) { busy(-1); return env; });
  };
  R.errText = function (command, env) {
    var er = (env && env.error) || {};
    return command + ': ' + (er.message || 'failed') + (er.hint ? ' (' + er.hint + ')' : '');
  };
  // Returns data or null (and toasts the error unless quiet).
  R.run = function (command, input, quiet) {
    return R.call(command, input).then(function (env) {
      if (env && env.ok) {
        if (!quiet) (env.warnings || []).forEach(function (w) { R.toast('warning: ' + w); });
        return env.data;
      }
      if (!quiet) R.toast(R.errText(command, env), 'bad');
      return null;
    });
  };
  R.getJson = function (url) {
    if (S.lost) return Promise.resolve(null);
    return fetch(url, { headers: { 'x-rustler-token': T } }).then(function (r) {
      if (r.status === 401) { R.sessionLost(); return null; }
      return r.ok && r.status !== 204 ? r.json() : null;
    }).catch(function () { return null; });
  };
  // The server restarted (new token): stop polling and say so once.
  R.sessionLost = function () {
    if (S.lost) return;
    S.lost = true;
    var el = R.toast('This page\'s session ended (the GUI server was restarted). Open the new URL printed by `rustler gui`.', 'bad');
    el.classList.add('sticky');
  };
  R.apiUrl = function (path, params) {
    var q = Object.keys(params || {}).filter(function (k) { return params[k] !== undefined && params[k] !== null && params[k] !== ''; })
      .map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(params[k]); });
    q.push('t=' + encodeURIComponent(T));
    if (S.project) q.push('project=' + encodeURIComponent(S.project));
    return path + '?' + q.join('&');
  };

  // ------------------------------------------------------------ jobs (background commands with progress)
  var J = R.jobs = { list: [], subs: [], timer: null, seen: {} };
  J.start = function (command, input) {
    var body = { command: command, input: input || {} };
    if (S.project) body.project = S.project;
    return post('/api/jobs', body).then(function (env) {
      if (!env || !env.ok) { R.toast(R.errText(command, env), 'bad'); return null; }
      J.poll();
      return env.data.job_id;
    });
  };
  J.cancel = function (id) { return post('/api/jobs/cancel', { job_id: id }).then(function () { J.poll(); }); };
  J.on = function (fn) { J.subs.push(fn); return function () { J.subs = J.subs.filter(function (f) { return f !== fn; }); }; };
  J.poll = function () {
    clearTimeout(J.timer);
    return R.getJson(R.apiUrl('/api/jobs')).then(function (d) {
      if (!d || !Array.isArray(d.jobs)) { if (!S.lost) J.timer = setTimeout(J.poll, 5000); return; }
      J.list = d.jobs;
      var running = J.list.filter(function (j) { return j.status === 'running'; });
      var chip = $('jobchip');
      if (chip) {
        chip.hidden = running.length === 0;
        if (running.length) {
          var j = running[0], p = j.progress || {};
          chip.textContent = running.length + ' job' + (running.length > 1 ? 's' : '') + ': ' + j.command +
            (p.pct !== null && p.pct !== undefined ? ' ' + Math.round(p.pct * 100) + '%' : '') + (p.stage ? ' (' + p.stage + ')' : '');
        }
      }
      J.list.forEach(function (j) {
        if (j.status !== 'running' && J.seen[j.id] === 'running') {
          if (j.status === 'ok') R.toast(j.command + ' finished', 'good');
          else R.toast(R.errText(j.command, j.result), 'bad');
        }
        J.seen[j.id] = j.status;
      });
      J.subs.forEach(function (f) { try { f(J.list); } catch (e) { console.warn(e); } });
      clearTimeout(J.timer);
      J.timer = setTimeout(J.poll, running.length ? 800 : 4000);
    });
  };

  // ------------------------------------------------------------ widgets
  R.button = function (label, fn, opts) {
    var o = opts || {};
    var b = h('button', { class: (o.primary ? 'primary ' : '') + (o.small ? 'small ' : '') + (o.cls || ''), title: o.title, type: 'button' }, label);
    var miss = o.need ? R.missing(o.need) : [];
    if (miss.length) { b.disabled = true; b.title = 'not available: ' + miss.join(', '); }
    b.addEventListener('click', function (ev) {
      if (b.disabled) return;
      b.disabled = true;
      Promise.resolve().then(function () { return fn(ev); }).catch(function (e) { R.toast(String(e), 'bad'); console.error(e); })
        .then(function () { b.disabled = false; });
    });
    return b;
  };
  R.img = function (path, cls) { return h('img', { class: cls || 'shot', src: R.fileUrl(path), alt: String(path).split(/[\\/]/).pop() }); };
  R.option = function (val, label, selected) { return h('option', { value: val, selected: selected }, label === undefined ? val : label); };
  R.select = function (items, value, onchange) {
    var s = h('select', null, items.map(function (it) {
      var v = Array.isArray(it) ? it[0] : it, l = Array.isArray(it) ? it[1] : it;
      return R.option(v, l, String(v) === String(value));
    }));
    if (onchange) s.addEventListener('change', function () { onchange(s.value); });
    return s;
  };
  R.field = function (label, el, hint) {
    return h('label', { class: 'field' }, h('span', { class: 'flabel' }, label), el, hint ? h('span', { class: 'fhint' }, hint) : null);
  };
  // Tabs: items [{id, label, render(el)}]; returns element. Remembers the selection under key.
  R.tabs = function (items, key, cls) {
    var bar = h('div', { class: 'tabbar ' + (cls || '') });
    var body = h('div', { class: 'tabbody' });
    var cur = R.store('rustler.tab.' + key) || (items[0] && items[0].id);
    if (!items.some(function (i) { return i.id === cur; })) cur = items[0] && items[0].id;
    function show(id) {
      cur = id; R.store('rustler.tab.' + key, id);
      Array.prototype.forEach.call(bar.children, function (b) { b.classList.toggle('active', b.getAttribute('data-id') === id); });
      clear(body);
      var it = items.filter(function (i) { return i.id === id; })[0];
      if (it) { try { it.render(body); } catch (e) { body.appendChild(h('div', { class: 'err' }, 'UI error: ' + e)); console.error(e); } }
    }
    items.forEach(function (it) {
      bar.appendChild(h('button', { class: 'tab', type: 'button', 'data-id': it.id, onclick: function () { show(it.id); }, title: it.title }, it.label));
    });
    var wrap = h('div', { class: 'tabs' }, bar, body);
    wrap.show = show;
    wrap.refresh = function () { show(cur); };
    wrap.current = function () { return cur; };
    if (cur) show(cur);
    return wrap;
  };
  R.modal = function (title, content, actions) {
    var back = h('div', { class: 'modal-back' });
    function close() { if (back.parentNode) back.parentNode.removeChild(back); }
    var box = h('div', { class: 'modal' }, h('div', { class: 'modal-head' }, h('strong', null, title), h('button', { class: 'small ghost', type: 'button', onclick: close }, 'Close')), content,
      actions ? h('div', { class: 'row end' }, actions) : null);
    back.appendChild(box);
    back.addEventListener('mousedown', function (e) { if (e.target === back) close(); });
    document.body.appendChild(back);
    return close;
  };

  // ------------------------------------------------------------ data helpers
  R.sid = function (s) { return s.id || s.source_id || ''; };
  R.cid = function (c) { return c.id || c.clip_id || ''; };
  R.loadStyles = function () {
    if (!R.has('caption.list_styles')) return Promise.resolve(R.FALLBACK_STYLES.map(function (n) { return { name: n }; }));
    return R.run('caption.list_styles', {}, true).then(function (d) {
      var items = R.listOf(d, 'styles', 'presets').map(function (x) { return typeof x === 'string' ? { name: x } : Object.assign({}, x, { name: x.name || x.id || x.preset }); })
        .filter(function (x) { return x.name; });
      return items.length ? items : R.FALLBACK_STYLES.map(function (n) { return { name: n }; });
    });
  };
  R.loadSources = function () {
    if (!R.has('source.list')) return Promise.resolve([]);
    return R.run('source.list', {}, true).then(function (d) { return R.listOf(d, 'sources'); });
  };

  // Renderer fonts for the live canvas preview (served from ~/.rustler/fonts by /api/font).
  R.fonts = { anton: 'Anton, Impact, sans-serif', poppins: '"Poppins XB", "Segoe UI", Arial, sans-serif' };
  R.loadFonts = function () {
    if (!window.FontFace || !document.fonts) return Promise.resolve();
    var list = [['Anton', 'anton', '400'], ['Poppins XB', 'poppins-extrabold', '800']];
    return Promise.all(list.map(function (f) {
      var ff = new FontFace(f[0], 'url(' + R.apiUrl('/api/font/' + f[1]) + ')', { weight: f[2] });
      return ff.load().then(function (x) { document.fonts.add(x); }).catch(function () { /* not installed: fallback fonts */ });
    }));
  };

  // ------------------------------------------------------------ router
  R.views = {};   // id -> {label, nav, render(el, arg)}
  R.NAV = ['project', 'sources', 'clips', 'editor', 'assets', 'brief', 'renders', 'activity', 'commands'];
  R.buildNav = function () {
    var nav = clear($('navlinks'));
    R.NAV.forEach(function (id) {
      var v = R.views[id]; if (!v) return;
      nav.appendChild(h('a', { 'data-v': id, class: S.view === id ? 'active' : '', title: v.label, onclick: function () { R.go(id, id === 'editor' ? (R.store('rustler.lastClip') || '') : ''); } },
        h('span', { class: 'ico' }, v.icon || v.label[0]), h('span', { class: 'lbl' }, v.label)));
    });
  };
  R.go = function (view, arg) {
    S.cleanup.forEach(function (f) { try { f(); } catch (e) { console.warn(e); } });
    S.cleanup = [];
    S.view = view; S.arg = arg || '';
    try { history.replaceState(null, '', location.pathname + location.search + '#' + view + (arg ? '/' + encodeURIComponent(arg) : '')); } catch (e) { /* ignore */ }
    R.buildNav();
    document.body.classList.toggle('editor-mode', view === 'editor');
    var v = clear($('view'));
    var spec = R.views[view] || R.views.project;
    try { spec.render(v, S.arg); } catch (e) { v.appendChild(h('div', { class: 'err' }, 'UI error: ' + e)); console.error(e); }
  };
  R.onLeave = function (fn) { S.cleanup.push(fn); };
  R.every = function (ms, fn) { var id = setInterval(fn, ms); R.onLeave(function () { clearInterval(id); }); return id; };
  R.setChip = function () {
    var p = S.projectInfo;
    var label = 'no project';
    if (p) label = (p.name || p.id || 'project') + (p.revision !== undefined ? '  rev ' + p.revision : '');
    $('projchip').textContent = label + (S.project ? '  |  ' + S.project : '');
  };
  R.refreshProject = function (quiet) {
    if (!R.has('project.show')) return Promise.resolve(null);
    return R.run('project.show', {}, quiet).then(function (d) {
      if (d) { S.projectInfo = (d.project && typeof d.project === 'object') ? Object.assign({}, d.project, { _raw: d }) : d; }
      else if (!quiet) S.projectInfo = null;
      R.setChip();
      return d;
    });
  };
  // ------------------------------------------------------------ shared helpers (pages + editor panels)
  // Caption style definitions by name (caption.styles: built-in + custom), for the live canvas.
  R.styleDefs = R.styleDefs || {};
  R.loadStyleDefs = function () {
    if (!R.has('caption.styles')) return Promise.resolve([]);
    return R.run('caption.styles', {}, true).then(function (d) {
      var list = (d && Array.isArray(d.styles)) ? d.styles : [];
      list.forEach(function (s) { R.styleDefs[s.name] = s; });
      return list;
    });
  };
  R.loadClips = function () {
    if (!R.has('clip.list')) return Promise.resolve([]);
    return R.run('clip.list', {}, true).then(function (d) { return R.listOf(d, 'clips'); });
  };
  R.badge = function (text, kind, title) { return h('span', { class: 'badge b-' + (kind || 'none'), title: title }, text); };
  R.statusBadge = function (st, label) {
    var s = String(st || '').toLowerCase();
    var k = /pass|ok|done|rendered|exported|yes/.test(s) ? 'pass' : /warn|stale|skip/.test(s) ? 'warn' : /fail|error|no/.test(s) ? 'fail' : 'none';
    return R.badge(label || st || '-', k);
  };
  // A small confirm dialog (no window.confirm so the page never blocks).
  R.confirm = function (title, text, okLabel) {
    return new Promise(function (res) {
      var close = null;
      var done = function (v) { if (close) close(); res(v); };
      close = R.modal(title, h('p', null, text), [
        h('button', { class: 'small ghost', type: 'button', onclick: function () { done(false); } }, 'Cancel'),
        h('button', { class: 'small primary', type: 'button', onclick: function () { done(true); } }, okLabel || 'OK')]);
    });
  };
  R.copyText = function (text) {
    var ok = function () { R.toast('Copied to the clipboard', 'good'); return true; };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text).then(ok).catch(function () { return fallback(); });
    }
    return Promise.resolve(fallback());
    function fallback() {
      var ta = h('textarea', { style: { position: 'fixed', left: '-9999px' } });
      ta.value = text; document.body.appendChild(ta); ta.select();
      var r = false; try { r = document.execCommand('copy'); } catch (e) { r = false; }
      document.body.removeChild(ta);
      if (r) return ok();
      R.toast('Copy failed: select the text and press Ctrl+C', 'bad');
      return false;
    }
  };

  // ------------------------------------------------------------ CLI lines -> commands
  // Commands print follow-ups / fixes as `rustler <group> <action> [positional] [--flag value]`.
  // parseCli maps such a line onto {command, input} with the same rules as the CLI (spec.positional,
  // kebab flags -> snake keys, JSON values unless the field is a string). Lines with <placeholders>
  // need a human value and return null.
  function tokens(s) {
    var out = [], cur = '', q = '', any = false;
    for (var i = 0; i < s.length; i++) {
      var c = s[i];
      if (q) {
        if (c === '\\' && q === '"' && s[i + 1] === '"') { cur += '"'; i++; continue; }
        if (c === q) { q = ''; continue; }
        cur += c; continue;
      }
      if (c === '"' || c === "'") { q = c; any = true; continue; }
      if (/\s/.test(c)) { if (cur || any) { out.push(cur); cur = ''; any = false; } continue; }
      cur += c;
    }
    if (cur || any) out.push(cur);
    return out;
  }
  R.cliTokens = tokens;
  R.parseCli = function (line) {
    var s = String(line || '').trim().replace(/\s{2,}\(.*\)\s*$/, '').replace(/^`|`$/g, '');
    var t = tokens(s);
    if (t[0] !== 'rustler') return null;
    t.shift();
    for (var i = 0; i < t.length; i++) if (t[i] === '--project') { t.splice(i, 2); i--; }
    if (t.length < 2) return null;
    var name = t[0] + '.' + t[1].replace(/-/g, '_');
    var spec = R.spec(name);
    if (!spec) return null;
    var rest = t.slice(2);
    if (rest.some(function (x) { return /^<.*>$/.test(x) || /<[a-z_]+>/.test(x); })) return null;
    var pos = [], input = {};
    function val(key, raw) {
      var k = R.schemaKind(name, key);
      if (k && k.kind === 'string') return raw;
      if (k && k.kind === 'array' && raw.charAt(0) !== '[') return raw.split(',').map(function (x) { return x.trim(); });
      try { return JSON.parse(raw); } catch (e) { return raw; }
    }
    for (var j = 0; j < rest.length; j++) {
      var tok = rest[j];
      if (tok.indexOf('--') === 0) {
        var key = tok.slice(2), v = null;
        var eq = key.indexOf('=');
        if (eq >= 0) { v = key.slice(eq + 1); key = key.slice(0, eq); }
        key = key.replace(/-/g, '_');
        if (v === null && j + 1 < rest.length && rest[j + 1].indexOf('--') !== 0) { v = rest[++j]; }
        if (v === null) {
          if (!R.hasProp(name, key) && key.indexOf('no_') === 0 && R.hasProp(name, key.slice(3))) input[key.slice(3)] = false;
          else input[key] = true;
        } else input[key] = val(key, v);
      } else pos.push(tok);
    }
    var slots = spec.positional || [];
    for (var p = 0; p < slots.length && p < pos.length; p++) {
      var last = p === slots.length - 1, kd = R.schemaKind(name, slots[p]);
      if (last && kd && kd.kind === 'array') { input[slots[p]] = pos.slice(p); break; }
      input[slots[p]] = val(slots[p], pos[p]);
    }
    if (pos.length > slots.length && !(slots.length && (R.schemaKind(name, slots[slots.length - 1]) || {}).kind === 'array')) return null;
    return { command: name, input: input };
  };
  // A "Run" button for a CLI line. opts.mutate(cmd, input) is used for clip edits in the editor.
  R.cliRun = function (line, opts) {
    var o = opts || {};
    var p = R.parseCli(line);
    if (!p) return h('span', { class: 'muted small', title: 'needs a value from you; run it in the CLI or the matching panel' }, 'manual');
    return R.button(o.label || 'Run', function () {
      var spec = R.spec(p.command) || {};
      var long = spec.annotations && spec.annotations.long_running;
      if (long && !o.mutate) {
        return R.jobs.start(p.command, p.input).then(function (jid) { if (jid) { R.toast(p.command + ' started'); if (o.done) R.waitJob ? R.waitJob(jid, o.done) : null; } });
      }
      var run = o.mutate && p.input.clip_id ? o.mutate(p.command, p.input) : R.run(p.command, p.input);
      return Promise.resolve(run).then(function (d) { if (d) { if (!o.mutate) R.toast(p.command + ' done', 'good'); if (o.done) o.done(d); } });
    }, { small: true, primary: !!o.primary, title: p.command + ' ' + JSON.stringify(p.input) });
  };
  // A CLI line with its Run button.
  R.cliRow = function (line, opts) {
    return h('div', { class: 'clirow' }, h('code', { class: 'mono small' }, String(line)), R.cliRun(line, opts));
  };
})();
