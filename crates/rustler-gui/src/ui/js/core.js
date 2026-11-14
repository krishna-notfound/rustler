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
  R.NAV = ['project', 'sources', 'clips', 'editor', 'renders', 'activity', 'commands'];
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
})();
