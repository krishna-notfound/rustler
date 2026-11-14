// Rustler GUI: plain JS, no build step. Every action is a registry command via POST /api/run.
(function () {
  'use strict';

  var T = new URLSearchParams(location.search).get('t') || '';
  var S = {
    cmds: {},            // name -> spec
    project: '',         // project dir typed by the user ('' = server default)
    projectInfo: null,
    view: 'project',
    sel: { source_id: '', start_unit: '', end_unit: '' },
    poll: null,
    busy: 0
  };
  var VIEWS = [
    ['project', 'Project'], ['sources', 'Sources'], ['clips', 'Clips'],
    ['renders', 'Renders'], ['activity', 'Activity'], ['commands', 'Commands']
  ];
  var LAYOUTS = ['center-crop', 'blur-fit', 'split-vertical'];
  var FALLBACK_STYLES = ['bold-karaoke', 'clean-white', 'boxed-highlight', 'hormozi-pop'];

  // ------------------------------------------------------------ helpers
  function h(tag, attrs) {
    var e = document.createElement(tag);
    var a = attrs || {};
    Object.keys(a).forEach(function (k) {
      var v = a[k];
      if (v === null || v === undefined || v === false) return;
      if (k === 'class') e.className = v;
      else if (k.indexOf('on') === 0) e.addEventListener(k.slice(2), v);
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
  function has(name) { return !!S.cmds[name]; }
  function fileUrl(p) { return '/api/file?path=' + encodeURIComponent(p) + '&t=' + encodeURIComponent(T); }
  function fmt(t) {
    t = Number(t);
    if (!isFinite(t)) return '-';
    var m = Math.floor(t / 60), s = t - m * 60;
    return m + ':' + (s < 10 ? '0' : '') + s.toFixed(1);
  }
  function short(v, n) {
    var s = typeof v === 'string' ? v : JSON.stringify(v);
    if (s === undefined) s = '';
    return s.length > (n || 80) ? s.slice(0, n || 80) + '...' : s;
  }
  function pretty(v) { try { return JSON.stringify(v, null, 2); } catch (e) { return String(v); } }
  function listOf(d) {
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
  }
  function store(k, v) { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch (e) { /* ignore */ } return null; }

  function toast(msg, kind) {
    var el = h('div', { class: 'toast ' + (kind || '') }, msg);
    $('toasts').appendChild(el);
    setTimeout(function () { if (el.parentNode) el.parentNode.removeChild(el); }, kind === 'bad' ? 9000 : 4000);
  }
  function busy(d) {
    S.busy = Math.max(0, S.busy + d);
    $('busy').hidden = S.busy === 0;
  }
  function na() {
    var names = Array.prototype.slice.call(arguments);
    return h('div', { class: 'na' }, 'Not available: ', names.join(', '), ' ', names.length > 1 ? 'are' : 'is',
      ' not registered in this build yet. Use the Commands page to see what exists.');
  }
  function missing(names) { return names.filter(function (n) { return !has(n); }); }

  // ------------------------------------------------------------ api
  function call(command, input) {
    busy(1);
    var body = { command: command, input: input || {} };
    if (S.project) body.project = S.project;
    return fetch('/api/run', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-rustler-token': T },
      body: JSON.stringify(body)
    }).then(function (r) { return r.json(); })
      .catch(function (e) { return { ok: false, error: { message: String(e), code: 'internal' } }; })
      .then(function (env) { busy(-1); return env; });
  }
  // Returns data or null (and toasts the error unless quiet).
  function run(command, input, quiet) {
    return call(command, input).then(function (env) {
      if (env && env.ok) {
        (env.warnings || []).forEach(function (w) { toast('warning: ' + w); });
        return env.data;
      }
      if (!quiet) {
        var er = (env && env.error) || {};
        toast(command + ': ' + (er.message || 'failed') + (er.hint ? ' (' + er.hint + ')' : ''), 'bad');
      }
      return null;
    });
  }
  function getJson(url) {
    return fetch(url, { headers: { 'x-rustler-token': T } }).then(function (r) { return r.ok ? r.json() : null; }).catch(function () { return null; });
  }
  function button(label, fn, opts) {
    var o = opts || {};
    var b = h('button', { class: (o.primary ? 'primary ' : '') + (o.small ? 'small' : ''), title: o.title }, label);
    if (o.need && missing(o.need).length) { b.disabled = true; b.title = 'not available: ' + missing(o.need).join(', '); }
    b.addEventListener('click', function () {
      if (b.disabled) return;
      b.disabled = true;
      Promise.resolve().then(fn).catch(function (e) { toast(String(e), 'bad'); }).then(function () { b.disabled = false; });
    });
    return b;
  }
  function img(path) { return h('img', { class: 'shot', src: fileUrl(path), alt: path }); }

  // ------------------------------------------------------------ nav
  function buildNav() {
    var nav = clear($('navlinks'));
    VIEWS.forEach(function (v) {
      var a = h('a', { 'data-v': v[0], class: S.view === v[0] ? 'active' : '', onclick: function () { go(v[0]); } }, v[1]);
      nav.appendChild(a);
    });
  }
  function go(view) {
    S.view = view;
    try { history.replaceState(null, '', location.pathname + location.search + '#' + view); } catch (e) { /* ignore */ }
    if (S.poll) { clearInterval(S.poll); S.poll = null; }
    buildNav();
    var v = clear($('view'));
    var fn = { project: viewProject, sources: viewSources, clips: viewClips, renders: viewRenders, activity: viewActivity, commands: viewCommands }[view];
    try { fn(v); } catch (e) { v.appendChild(h('div', { class: 'err' }, 'UI error: ' + e)); }
  }
  function setChip() {
    var p = S.projectInfo;
    var label = 'no project';
    if (p) label = (p.name || p.id || 'project') + (p.revision !== undefined ? '  rev ' + p.revision : '');
    $('projchip').textContent = label + (S.project ? '  |  ' + S.project : '');
  }

  // ------------------------------------------------------------ project
  function refreshProject(quiet) {
    if (!has('project.show')) return Promise.resolve(null);
    return run('project.show', {}, quiet).then(function (d) {
      if (d) { S.projectInfo = (d.project && typeof d.project === 'object') ? Object.assign({}, d.project, { _raw: d }) : d; }
      else if (!quiet) S.projectInfo = null;
      setChip();
      return d;
    });
  }
  function viewProject(v) {
    v.appendChild(h('h1', null, 'Project'));
    var path = h('input', { class: 'grow', placeholder: 'path to project folder, e.g. C:\\work\\my-project', value: S.project });
    var info = h('div');
    function showInfo() {
      clear(info);
      var d = S.projectInfo;
      if (!d) { info.appendChild(h('p', { class: 'muted' }, 'No project open yet. Enter a folder and press Open (existing) or Create (new).')); return; }
      var raw = d._raw || d;
      var p = raw.project || raw;
      var counts = raw.counts || {};
      var nSrc = counts.sources !== undefined ? counts.sources : (Array.isArray(p.sources) ? p.sources.length : '?');
      var nClip = counts.clips !== undefined ? counts.clips : (Array.isArray(p.clips) ? p.clips.length : '?');
      info.appendChild(h('div', { class: 'card' },
        h('h2', null, p.name || 'project'),
        h('div', { class: 'muted small mono' }, p.id || ''),
        h('div', { class: 'row' }, h('span', null, 'Sources: ' + nSrc), h('span', null, 'Clips: ' + nClip), h('span', null, 'Revision: ' + (p.revision === undefined ? '?' : p.revision))),
        h('div', { class: 'row' },
          button('Sources', function () { go('sources'); }), button('Clips', function () { go('clips'); })),
        h('details', null, h('summary', null, 'raw project.show'), h('pre', null, pretty(raw)))));
    }
    function open() {
      S.project = path.value.trim();
      store('rustler.project', S.project);
      return refreshProject(false).then(function (d) { if (d) toast('Project opened', 'good'); showInfo(); });
    }
    function create() {
      var dir = path.value.trim();
      if (!dir) { toast('Enter a folder path first', 'bad'); return; }
      var name = dir.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || 'project';
      S.project = dir;
      store('rustler.project', S.project);
      return run('project.create', { name: name, dir: dir }).then(function (d) {
        if (d) { toast('Project created', 'good'); return refreshProject(true).then(showInfo); }
      });
    }
    var miss = missing(['project.show']);
    v.appendChild(h('div', { class: 'card' },
      h('div', { class: 'row' }, path,
        button('Open', open, { primary: true, need: ['project.show'] }),
        button('Create', create, { need: ['project.create'] })),
      h('div', { class: 'muted small' }, 'Leave empty to use the project the server was started with.')));
    if (miss.length) v.appendChild(na.apply(null, miss));
    v.appendChild(info);
    showInfo();
  }

  // ------------------------------------------------------------ sources
  function sid(s) { return s.id || s.source_id || ''; }
  function extractUnits(d) {
    var arr = listOf(d, 'units', 'structured', 'matches', 'results').filter(function (u) { return u && typeof u === 'object' && u.text !== undefined; });
    return arr.map(function (u) {
      return { id: u.id !== undefined ? u.id : u.unit_id, start: u.start, end: u.end, text: u.text };
    });
  }
  function viewSources(v) {
    v.appendChild(h('h1', null, 'Sources'));
    var miss = missing(['source.list']);
    if (miss.length) { v.appendChild(na.apply(null, miss)); return; }
    var input = h('input', { class: 'grow', placeholder: 'file path or URL' });
    var sections = h('input', { placeholder: 'sections e.g. 01:00-02:30, 10:00-11:00 (URL only, optional)', style: 'min-width:260px' });
    var listBox = h('div');
    v.appendChild(h('div', { class: 'card' },
      h('h2', null, 'Import'),
      h('div', { class: 'row' }, input, sections,
        button('Import', function () {
          var val = input.value.trim();
          if (!val) { toast('Enter a path or URL', 'bad'); return; }
          var body = { input: val };
          var secs = sections.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
          if (secs.length) body.sections = secs;
          return run('source.import', body).then(function (d) { if (d) { toast('Imported', 'good'); input.value = ''; loadList(); } });
        }, { primary: true, need: ['source.import'] }))));
    v.appendChild(listBox);

    function loadList() {
      run('source.list', {}).then(function (d) {
        clear(listBox);
        var items = listOf(d, 'sources');
        if (!items.length) { listBox.appendChild(h('p', { class: 'muted' }, 'No sources yet. Import a file or URL above.')); return; }
        items.forEach(function (s) { listBox.appendChild(sourceCard(s)); });
      });
    }
    loadList();
  }
  function sourceCard(s) {
    var id = sid(s);
    var pr = s.probe || {};
    var meta = [];
    if (pr.duration !== undefined) meta.push(fmt(pr.duration));
    if (pr.width) meta.push(pr.width + 'x' + pr.height);
    if (pr.has_audio === false) meta.push('no audio');
    if (pr.has_video === false) meta.push('audio only');
    var panel = h('div', { hidden: true });
    var out = h('div');
    var unitsBox = h('div');
    var q = h('input', { class: 'grow', placeholder: 'search transcript' });
    var allUnits = [];

    function unitRow(u) {
      var row = h('div', { class: 'unit' });
      var sel = S.sel.source_id === id;
      if (sel && String(S.sel.start_unit) === String(u.id)) row.className += ' s';
      if (sel && String(S.sel.end_unit) === String(u.id)) row.className += ' e';
      row.appendChild(h('span', { class: 't' }, '#' + u.id));
      row.appendChild(h('span', { class: 't' }, fmt(u.start) + '-' + fmt(u.end)));
      row.appendChild(h('span', { class: 'x' }, u.text));
      row.appendChild(h('span', { class: 'b' },
        button('tiles', function () { return tiles(u); }, { small: true, need: ['see.tiles'] }),
        button('start', function () { setSel('start_unit', u.id); }, { small: true }),
        button('end', function () { setSel('end_unit', u.id); }, { small: true })));
      return row;
    }
    function setSel(k, val) {
      if (S.sel.source_id !== id) S.sel = { source_id: id, start_unit: '', end_unit: '' };
      S.sel[k] = val;
      drawUnits(allUnits);
      toast('Clip range: #' + (S.sel.start_unit === '' ? '?' : S.sel.start_unit) + ' to #' + (S.sel.end_unit === '' ? '?' : S.sel.end_unit) + ' (create it on the Clips page)');
    }
    function drawUnits(units) {
      clear(unitsBox);
      if (!units.length) { unitsBox.appendChild(h('p', { class: 'muted' }, 'No transcript units. Run "Analyze quick" first (or the transcript is empty).')); return; }
      var box = h('div', { class: 'units' });
      units.forEach(function (u) { box.appendChild(unitRow(u)); });
      unitsBox.appendChild(box);
    }
    function loadTranscript() {
      return run('transcript.get', { source_id: id }).then(function (d) {
        if (!d) return;
        allUnits = extractUnits(d);
        if (!allUnits.length && Array.isArray(d.lines)) {
          clear(unitsBox).appendChild(h('pre', null, d.lines.join('\n')));
        } else drawUnits(allUnits);
      });
    }
    function search() {
      var term = q.value.trim();
      if (!term) { drawUnits(allUnits); return; }
      return run('transcript.search', { source_id: id, query: term, limit: 20 }).then(function (d) {
        if (d) drawUnits(extractUnits(d));
      });
    }
    function tiles(u) {
      return run('see.tiles', { source_id: id, start: Math.max(0, Number(u.start) - 6), end: Number(u.end) + 6, count: 12 }).then(function (d) {
        if (!d) return;
        clear(out);
        out.appendChild(h('div', { class: 'muted small' }, 'Tiles around unit #' + u.id + ' (' + fmt(u.start) + ')'));
        if (d.image) out.appendChild(img(d.image)); else out.appendChild(h('pre', null, pretty(d)));
      });
    }
    var card = h('div', { class: 'card' },
      h('h2', null, s.name || id),
      h('div', { class: 'muted small mono' }, id + (meta.length ? '  |  ' + meta.join('  |  ') : '')),
      h('div', { class: 'row' },
        button('Analyze quick', function () {
          toast('Analyzing... this can take a while');
          return run('analyze.quick', { source_id: id }).then(function (d) {
            if (d) { toast('Analysis done', 'good'); clear(out).appendChild(h('pre', null, pretty(d))); return loadTranscript(); }
          });
        }, { primary: true, need: ['analyze.quick'] }),
        button('Transcript', function () { panel.hidden = false; return loadTranscript(); }, { need: ['transcript.get'] })),
      panel, out);
    panel.appendChild(h('div', { class: 'row' }, q,
      button('Search', search, { need: ['transcript.search'] }),
      button('Clear', function () { q.value = ''; drawUnits(allUnits); })));
    panel.appendChild(unitsBox);
    return card;
  }

  // ------------------------------------------------------------ clips
  function cid(c) { return c.id || c.clip_id || ''; }
  function option(val, label, selected) { return h('option', { value: val, selected: selected }, label === undefined ? val : label); }
  function loadStyles() {
    if (!has('caption.list_styles')) return Promise.resolve(FALLBACK_STYLES);
    return run('caption.list_styles', {}, true).then(function (d) {
      var items = listOf(d, 'styles', 'presets').map(function (x) { return typeof x === 'string' ? x : (x.name || x.id || x.preset); }).filter(Boolean);
      return items.length ? items : FALLBACK_STYLES;
    });
  }
  function viewClips(v) {
    v.appendChild(h('h1', null, 'Clips'));
    var miss = missing(['clip.list']);
    if (miss.length) { v.appendChild(na.apply(null, miss)); return; }
    var form = h('div', { class: 'card' });
    var list = h('div');
    v.appendChild(form);
    v.appendChild(list);

    Promise.all([has('source.list') ? run('source.list', {}, true) : null, loadStyles()]).then(function (res) {
      var sources = listOf(res[0], 'sources');
      var styles = res[1];
      var name = h('input', { placeholder: 'clip name', value: 'Clip 1' });
      var src = h('select', null, sources.map(function (s) { return option(sid(s), (s.name || sid(s)), S.sel.source_id === sid(s)); }));
      var su = h('input', { type: 'number', min: 0, style: 'width:90px', placeholder: 'start #', value: S.sel.start_unit });
      var eu = h('input', { type: 'number', min: 0, style: 'width:90px', placeholder: 'end #', value: S.sel.end_unit });
      var ss = h('input', { type: 'number', step: '0.1', style: 'width:100px', placeholder: 'start s' });
      var es = h('input', { type: 'number', step: '0.1', style: 'width:100px', placeholder: 'end s' });
      var lay = h('select', null, LAYOUTS.map(function (l) { return option(l); }));
      var sty = h('select', null, [option('', 'default captions')].concat(styles.map(function (x) { return option(x); })));
      form.appendChild(h('h2', null, 'Create clip'));
      form.appendChild(h('div', { class: 'row' }, name, src));
      form.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted' }, 'units'), su, eu,
        h('span', { class: 'muted' }, 'or seconds'), ss, es));
      form.appendChild(h('div', { class: 'row' }, lay, has('caption.list_styles') || styles.length ? sty : null,
        button('Create clip', function () {
          var b = { name: name.value.trim() || 'clip', source_id: src.value, layout: lay.value, captions: true };
          if (!b.source_id) { toast('Import a source first', 'bad'); return; }
          if (su.value !== '') b.start_unit = Number(su.value);
          if (eu.value !== '') b.end_unit = Number(eu.value);
          if (ss.value !== '') b.start = Number(ss.value);
          if (es.value !== '') b.end = Number(es.value);
          return run('clip.create', b).then(function (d) {
            if (!d) return;
            var c = d.clip || d;
            toast('Clip created', 'good');
            var p = (sty.value && has('clip.set_captions') && cid(c)) ? run('clip.set_captions', { clip_id: cid(c), preset: sty.value }) : Promise.resolve();
            return p.then(loadClips);
          });
        }, { primary: true, need: ['clip.create'] })));
      if (!sources.length) form.appendChild(h('div', { class: 'muted small' }, 'No sources yet, import one on the Sources page.'));
      form.appendChild(h('div', { class: 'muted small' }, 'Tip: pick units on the Sources page (start / end buttons) and they appear here.'));
      S._styles = styles;
    });

    function loadClips() {
      return run('clip.list', {}).then(function (d) {
        clear(list);
        var items = listOf(d, 'clips');
        if (!items.length) { list.appendChild(h('p', { class: 'muted' }, 'No clips yet.')); return; }
        items.forEach(function (c) { list.appendChild(clipCard(c)); });
      });
    }
    loadClips();
  }
  function clipCard(c0) {
    var id = cid(c0);
    var card = h('div', { class: 'card' });
    var out = h('div');
    var body = h('div', null, h('span', { class: 'muted' }, 'loading...'));
    card.appendChild(h('h2', null, c0.name || id));
    card.appendChild(h('div', { class: 'muted small mono' }, id));
    card.appendChild(body);
    card.appendChild(out);
    if (!has('clip.show')) { clear(body).appendChild(na('clip.show')); return card; }

    function load() {
      return run('clip.show', { clip_id: id }).then(function (d) {
        if (!d) { clear(body).appendChild(h('span', { class: 'err' }, 'could not load')); return; }
        draw(d);
      });
    }
    function draw(d) {
      clear(body);
      var c = d.clip && typeof d.clip === 'object' ? d.clip : d;
      var segs = listOf(c.segments ? c : d, 'segments');
      var outs = Array.isArray(d.segments) ? d.segments : (d.timemap && Array.isArray(d.timemap.segments) ? d.timemap.segments : []);
      var dur = d.duration !== undefined ? d.duration : (d.total_duration !== undefined ? d.total_duration : (d.timemap && d.timemap.duration));
      var caps = c.captions || {};
      var firstPreset = segs[0] && segs[0].layout && segs[0].layout.preset;
      body.appendChild(h('div', { class: 'muted small' }, (dur !== undefined && dur !== null ? 'duration ' + fmt(dur) + '  |  ' : '') + segs.length + ' segment(s)'));
      var tbody = h('tbody');
      segs.forEach(function (sg, i) {
        var o = outs[i] || {};
        var st = sg.start !== undefined ? sg.start : o.start;
        var en = sg.end !== undefined ? sg.end : o.end;
        tbody.appendChild(h('tr', null,
          h('td', { class: 'mono' }, sg.id || sg.segment_id || ''),
          h('td', { class: 'mono' }, sg.source_id || ''),
          h('td', null, fmt(st) + ' - ' + fmt(en)),
          h('td', null, (sg.layout && sg.layout.preset) || o.layout || '-'),
          h('td', null, o.out_start !== undefined ? fmt(o.out_start) + ' - ' + fmt(o.out_end) : '')));
      });
      if (segs.length) body.appendChild(h('table', null, h('thead', null, h('tr', null, h('th', null, 'segment'), h('th', null, 'source'), h('th', null, 'source time'), h('th', null, 'layout'), h('th', null, 'output time'))), tbody));

      var lay = h('select', null, LAYOUTS.map(function (l) { return option(l, l, l === firstPreset); }));
      var capOn = h('input', { type: 'checkbox', checked: caps.enabled !== false });
      var styles = (S._styles && S._styles.length) ? S._styles : FALLBACK_STYLES;
      var sty = h('select', null, styles.map(function (x) { return option(x, x, x === caps.preset); }));
      var outT = h('input', { type: 'number', step: '0.1', min: 0, value: 1, style: 'width:80px', title: 'output time (s) for preview' });

      body.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted' }, 'Layout'), lay,
        button('Apply', function () {
          return run('clip.set_layout', { clip_id: id, preset: lay.value }).then(function (r) { if (r) { toast('Layout set', 'good'); return load(); } });
        }, { small: true, need: ['clip.set_layout'] })));
      body.appendChild(h('div', { class: 'row' }, h('label', null, capOn, ' Captions'), sty,
        button('Apply', function () {
          return run('clip.set_captions', { clip_id: id, enabled: capOn.checked, preset: sty.value }).then(function (r) { if (r) { toast('Captions updated', 'good'); return load(); } });
        }, { small: true, need: ['clip.set_captions'] })));
      body.appendChild(h('div', { class: 'row' },
        button('Render', function () {
          toast('Rendering...');
          return run('render.run', { clip_id: id }).then(function (r) {
            if (!r) return;
            toast('Render finished', 'good');
            clear(out).appendChild(h('div', null, 'Rendered ', h('span', { class: 'mono' }, r.path || ''), ' ',
              button('Open Renders', function () { S.renderClip = id; go('renders'); }, { small: true })));
            return load();
          });
        }, { primary: true, need: ['render.run'] }),
        h('span', { class: 'muted small' }, 'preview at'), outT,
        button('Preview frame', function () {
          return run('render.preview_frame', { clip_id: id, out_t: Number(outT.value) || 0 }).then(function (r) {
            if (r) clear(out).appendChild(r.image ? img(r.image) : h('pre', null, pretty(r)));
          });
        }, { need: ['render.preview_frame'] }),
        button('QC', function () {
          return run('qc.run', { clip_id: id }).then(function (r) {
            if (r) { var box = clear(out); renderQc(box, r); }
          });
        }, { need: ['qc.run'] }),
        button('Export', function () {
          return run('export.clip', { clip_id: id }).then(function (r) {
            if (r) { toast('Exported', 'good'); clear(out).appendChild(h('pre', null, pretty(r))); }
          });
        }, { need: ['export.clip'] })));
    }
    load();
    return card;
  }

  // ------------------------------------------------------------ renders / qc
  function renderQc(box, rep) {
    var checks = listOf(rep.report || rep, 'checks');
    var r = rep.report || rep;
    box.appendChild(h('h3', null, 'QC ' + (r.passed === true ? '- passed' : r.passed === false ? '- failed' : '')));
    if (checks.length) {
      box.appendChild(h('table', null,
        h('thead', null, h('tr', null, h('th', null, 'check'), h('th', null, 'status'), h('th', null, 'value'), h('th', null, 'expected'), h('th', null, 'hint'))),
        h('tbody', null, checks.map(function (c) {
          var st = String(c.status || '').toLowerCase();
          return h('tr', null, h('td', null, c.name), h('td', { class: 'st-' + st }, st),
            h('td', { class: 'mono' }, short(c.value, 60)), h('td', null, c.expected || ''), h('td', { class: 'muted' }, c.fix_hint || ''));
        }))));
    } else box.appendChild(h('pre', null, pretty(rep)));
    if (r.contact_sheet) { box.appendChild(h('h3', null, 'Contact sheet')); box.appendChild(img(r.contact_sheet)); }
  }
  function viewRenders(v) {
    v.appendChild(h('h1', null, 'Renders'));
    var miss = missing(['clip.list', 'clip.show']);
    if (miss.length) { v.appendChild(na.apply(null, miss)); return; }
    var sel = h('select');
    var rsel = h('select');
    var stage = h('div');
    v.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted' }, 'Clip'), sel, h('span', { class: 'muted' }, 'Render'), rsel));
    v.appendChild(stage);
    var current = null;

    function showRender(r) {
      clear(stage);
      if (!r) { stage.appendChild(h('p', { class: 'muted' }, 'No renders for this clip yet. Use Render on the Clips page.')); return; }
      stage.appendChild(h('div', { class: 'card' },
        h('video', { controls: true, preload: 'metadata', src: fileUrl(r.path) }),
        h('div', { class: 'muted small mono' }, r.path + '  |  ' + (r.encoder || '') + '  |  ' + (r.quality || ''))));
      var qcBox = h('div', { class: 'card' });
      stage.appendChild(qcBox);
      function drawQc(rep) { clear(qcBox); if (rep) renderQc(qcBox, rep); }
      qcBox.appendChild(h('div', { class: 'row' }, button('Run QC', function () {
        return run('qc.run', { clip_id: current && cid(current) }).then(drawQc);
      }, { primary: true, need: ['qc.run'] }), h('span', { class: 'muted small' }, r.qc ? 'loading saved QC...' : 'no QC yet for this render')));
      if (r.qc) getJson(fileUrl(r.qc)).then(function (rep) { if (rep) drawQc(rep); else clear(qcBox).appendChild(h('div', { class: 'muted' }, 'QC report not readable.')); });
    }
    function pickClip(id) {
      return run('clip.show', { clip_id: id }).then(function (d) {
        if (!d) return;
        var c = d.clip && typeof d.clip === 'object' ? d.clip : d;
        current = c;
        if (!c.id && !c.clip_id) c.id = id;
        var rs = Array.isArray(c.renders) ? c.renders : (Array.isArray(d.renders) ? d.renders : []);
        clear(rsel);
        rs.forEach(function (r, i) { rsel.appendChild(option(String(i), (r.id || 'render') + '  ' + (r.created_at || ''), i === rs.length - 1)); });
        rsel.onchange = function () { showRender(rs[Number(rsel.value)]); };
        showRender(rs[rs.length - 1]);
      });
    }
    run('clip.list', {}).then(function (d) {
      var items = listOf(d, 'clips');
      if (!items.length) { stage.appendChild(h('p', { class: 'muted' }, 'No clips yet.')); return; }
      items.forEach(function (c) { sel.appendChild(option(cid(c), c.name || cid(c), cid(c) === S.renderClip)); });
      sel.onchange = function () { pickClip(sel.value); };
      pickClip(sel.value);
    });
  }

  // ------------------------------------------------------------ activity
  function viewActivity(v) {
    v.appendChild(h('h1', null, 'Activity'));
    v.appendChild(h('p', { class: 'muted small' }, 'Live feed of events.ndjson (agent and human actions), refreshed every 2 s.'));
    var box = h('div', { class: 'card' });
    v.appendChild(box);
    function tick() {
      var url = '/api/events?t=' + encodeURIComponent(T) + (S.project ? '&project=' + encodeURIComponent(S.project) : '');
      getJson(url).then(function (evs) {
        if (S.view !== 'activity' || !Array.isArray(evs)) return;
        clear(box);
        if (!evs.length) { box.appendChild(h('p', { class: 'muted' }, 'No events yet.')); return; }
        evs.slice().reverse().forEach(function (e) {
          var actor = String(e.actor || 'system').toLowerCase();
          var time = String(e.ts || '').replace('T', ' ').slice(0, 19);
          box.appendChild(h('div', { class: 'ev' },
            h('span', { class: 'muted mono small' }, time),
            h('span', { class: 'badge ' + actor }, actor),
            h('strong', null, e.kind || ''),
            h('span', { class: 'd', title: short(e.data, 400) }, short(e.data, 160))));
        });
      });
    }
    tick();
    S.poll = setInterval(tick, 2000);
  }

  // ------------------------------------------------------------ commands
  function viewCommands(v) {
    v.appendChild(h('h1', null, 'Commands'));
    var names = Object.keys(S.cmds).sort();
    var filter = h('input', { class: 'grow', placeholder: 'filter ' + names.length + ' commands' });
    var box = h('div');
    v.appendChild(h('div', { class: 'row' }, filter));
    v.appendChild(h('p', { class: 'muted small' }, 'Every registry command is available here as a JSON form, the same input the CLI takes.'));
    v.appendChild(box);
    function draw() {
      clear(box);
      var f = filter.value.trim().toLowerCase();
      names.filter(function (n) { return !f || n.indexOf(f) >= 0 || String(S.cmds[n].summary || '').toLowerCase().indexOf(f) >= 0; })
        .forEach(function (n) { box.appendChild(commandItem(S.cmds[n])); });
    }
    filter.addEventListener('input', draw);
    draw();
  }
  function commandItem(spec) {
    var ta = h('textarea', { spellcheck: 'false' });
    ta.value = '{}';
    var out = h('pre', { hidden: true });
    var props = (spec.input_schema && spec.input_schema.properties) || {};
    var req = (spec.input_schema && spec.input_schema.required) || [];
    var rows = Object.keys(props).map(function (k) {
      var p = props[k] || {};
      var ty = p.type ? (Array.isArray(p.type) ? p.type.join('|') : p.type) : (p.$ref ? String(p.$ref).split('/').pop() : '');
      return h('tr', null, h('td', { class: 'mono' }, k + (req.indexOf(k) >= 0 ? ' *' : '')), h('td', { class: 'mono' }, ty), h('td', { class: 'muted' }, p.description || ''));
    });
    var a = spec.annotations || {};
    var flags = Object.keys(a).filter(function (k) { return a[k]; }).join(', ');
    var examples = (spec.examples || []).map(function (e) { return h('div', { class: 'small muted' }, e.description + ': ', h('span', { class: 'mono' }, e.cli)); });
    return h('details', null,
      h('summary', null, h('span', { class: 'n' }, spec.name), h('span', { class: 'muted' }, spec.summary)),
      h('div', { class: 'body' },
        flags ? h('div', { class: 'small muted' }, 'flags: ' + flags) : null,
        (spec.positional && spec.positional.length) ? h('div', { class: 'small muted' }, 'cli positional: ' + spec.positional.join(' ')) : null,
        rows.length ? h('table', null, h('thead', null, h('tr', null, h('th', null, 'input field (* required)'), h('th', null, 'type'), h('th', null, ''))), h('tbody', null, rows)) : null,
        examples,
        ta,
        h('div', { class: 'row' }, button('Run', function () {
          var parsed;
          try { parsed = JSON.parse(ta.value || '{}'); } catch (e) { toast('Invalid JSON: ' + e.message, 'bad'); return; }
          return call(spec.name, parsed).then(function (env) {
            out.hidden = false;
            out.textContent = pretty(env);
            // show any image/video path in the result
            var m = JSON.stringify(env).match(/"([^"]+\.(?:jpg|jpeg|png))"/i);
            var prev = out.nextSibling; if (prev && prev.className === 'shotwrap') prev.parentNode.removeChild(prev);
            if (m && env.ok) { var w = h('div', { class: 'shotwrap' }, img(m[1].replace(/\\\\/g, '\\'))); out.parentNode.insertBefore(w, out.nextSibling); }
          });
        }, { primary: true })),
        out));
  }

  // ------------------------------------------------------------ boot
  function boot() {
    // Read the requested view before anything rewrites the hash (refreshProject may re-render).
    var initialView = (location.hash || '').replace('#', '');
    window.addEventListener('error', function (e) { toast('UI error: ' + e.message, 'bad'); });
    window.addEventListener('unhandledrejection', function (e) { toast('UI error: ' + e.reason, 'bad'); });
    var saved = store('rustler.project');
    if (saved) S.project = saved;
    buildNav();
    getJson('/api/commands?t=' + encodeURIComponent(T)).then(function (list) {
      (Array.isArray(list) ? list : []).forEach(function (c) { S.cmds[c.name] = c; });
      return refreshProject(true);
    }).then(function (d) {
      if (!d && S.project) { /* saved path may be stale; leave it, the user can re-open */ }
      var ok = VIEWS.some(function (x) { return x[0] === initialView; });
      go(ok ? initialView : 'project');
    });
  }
  boot();
})();
