// Sources page: import, analyze, transcript (pick units), tiles, highlight candidates, understanding.
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  R.extractUnits = function (d) {
    var arr = R.listOf(d, 'units', 'structured', 'matches', 'results').filter(function (u) { return u && typeof u === 'object' && u.text !== undefined; });
    return arr.map(function (u) { return { id: u.id !== undefined ? u.id : u.unit_id, start: u.start, end: u.end, text: u.text }; });
  };

  function render(v) {
    v.appendChild(h('h1', null, 'Sources'));
    var miss = R.missing(['source.list']);
    if (miss.length) { v.appendChild(R.na.apply(null, miss)); return; }
    var input = h('input', { class: 'grow', placeholder: 'file path or URL' });
    var sections = h('input', { placeholder: 'sections e.g. 01:00-02:30, 10:00-11:00 (URL only, optional)', style: 'min-width:260px' });
    var listBox = h('div');
    v.appendChild(h('div', { class: 'card' },
      h('h2', null, 'Import'),
      h('div', { class: 'row' }, input, sections,
        R.button('Import', function () {
          var val = input.value.trim();
          if (!val) { R.toast('Enter a path or URL', 'bad'); return; }
          var body = { input: val };
          var secs = sections.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
          if (secs.length) body.sections = secs;
          return R.jobs.start('source.import', body).then(function (id) { if (id) { R.toast('Importing in the background...'); input.value = ''; waitJob(id, loadList); } });
        }, { primary: true, need: ['source.import'] }))));
    v.appendChild(listBox);
    function loadList() {
      R.run('source.list', {}).then(function (d) {
        R.clear(listBox);
        var items = R.listOf(d, 'sources');
        if (!items.length) { listBox.appendChild(h('p', { class: 'muted' }, 'No sources yet. Import a file or URL above.')); return; }
        items.forEach(function (s) { listBox.appendChild(sourceCard(s)); });
      });
    }
    loadList();
  }
  function waitJob(id, then) {
    var off = R.jobs.on(function (list) {
      var j = list.filter(function (x) { return x.id === id; })[0];
      if (j && j.status !== 'running') { off(); if (j.status === 'ok' && then) then(j.result.data); }
    });
    R.onLeave(off);
  }
  R.waitJob = waitJob;

  function sourceCard(s) {
    var id = R.sid(s);
    var pr = s.probe || { duration: s.duration, has_audio: s.has_audio, has_video: s.has_video };
    var meta = [];
    if (s.transcribed !== undefined) meta.push(s.transcribed ? 'transcribed' : 'not analyzed yet');
    if (pr.duration !== undefined) meta.push(R.fmt(pr.duration));
    if (pr.width) meta.push(pr.width + 'x' + pr.height);
    if (pr.fps) meta.push(R.round(pr.fps, 2) + ' fps');
    if (pr.has_audio === false) meta.push('no audio');
    if (pr.has_video === false) meta.push('audio only');
    var body = h('div', { class: 'srcbody', hidden: true });
    var thumb = (pr.has_video !== false && R.has('see.frame') && pr.duration) ? h('img', { class: 'thumb', loading: 'lazy', src: R.frameUrl(id, Math.min(pr.duration * 0.3, 30), 320), alt: '' }) : h('div', { class: 'thumb empty' });
    var tabsHost = h('div');
    var built = false;
    function toggle() {
      body.hidden = !body.hidden;
      if (!body.hidden && !built) { built = true; tabsHost.appendChild(sourceTabs(s)); }
    }
    var card = h('div', { class: 'card src' },
      h('div', { class: 'srchead' }, thumb,
        h('div', { class: 'grow' },
          h('h2', null, s.name || id),
          h('div', { class: 'muted small mono' }, id + (meta.length ? '  |  ' + meta.join('  |  ') : '')),
          h('div', { class: 'row' },
            R.button('Analyze quick', function () {
              return R.jobs.start('analyze.quick', { source_id: id }).then(function (jid) {
                if (jid) { R.toast('Analyzing in the background (see the job chip)'); waitJob(jid, function () { if (built) { R.clear(tabsHost); tabsHost.appendChild(sourceTabs(s)); } }); }
              });
            }, { primary: true, need: ['analyze.quick'] }),
            R.button('Open', toggle)))),
      body);
    body.appendChild(tabsHost);
    return card;
  }

  function sourceTabs(s) {
    var items = [
      { id: 'transcript', label: 'Transcript', render: function (el) { transcriptTab(el, s); } },
      { id: 'highlights', label: 'Highlights', render: function (el) { highlightsTab(el, s); } },
      { id: 'understand', label: 'Understand', render: function (el) { understandTab(el, s); } },
      { id: 'info', label: 'Info', render: function (el) { infoTab(el, s); } }
    ];
    return R.tabs(items, 'source');
  }

  function transcriptTab(el, s) {
    var id = R.sid(s);
    if (!R.has('transcript.get')) { el.appendChild(R.na('transcript.get')); return; }
    var q = h('input', { class: 'grow', placeholder: 'search transcript' });
    var unitsBox = h('div');
    var out = h('div');
    var allUnits = [];
    function unitRow(u) {
      var sel = S.sel.source_id === id;
      var row = h('div', { class: 'unit' + (sel && String(S.sel.start_unit) === String(u.id) ? ' s' : '') + (sel && String(S.sel.end_unit) === String(u.id) ? ' e' : '') });
      row.appendChild(h('span', { class: 't' }, '#' + u.id));
      row.appendChild(h('span', { class: 't' }, R.fmt(u.start) + '-' + R.fmt(u.end)));
      row.appendChild(h('span', { class: 'x' }, u.text));
      row.appendChild(h('span', { class: 'b' },
        R.button('tiles', function () { return tiles(u); }, { small: true, need: ['see.tiles'] }),
        R.button('start', function () { setSel('start_unit', u.id); }, { small: true }),
        R.button('end', function () { setSel('end_unit', u.id); }, { small: true })));
      return row;
    }
    function setSel(k, val) {
      if (S.sel.source_id !== id) S.sel = { source_id: id, start_unit: '', end_unit: '' };
      S.sel[k] = val;
      drawUnits(allUnits);
      drawSel();
    }
    var selBar = h('div', { class: 'row selbar' });
    function drawSel() {
      R.clear(selBar);
      if (S.sel.source_id !== id || (S.sel.start_unit === '' && S.sel.end_unit === '')) { selBar.appendChild(h('span', { class: 'muted small' }, 'Pick a start and an end unit to create a clip.')); return; }
      var name = h('input', { placeholder: 'clip name', value: 'Clip ' + (S.sel.start_unit === '' ? '' : '#' + S.sel.start_unit) });
      selBar.appendChild(h('span', null, 'Range #' + (S.sel.start_unit === '' ? '?' : S.sel.start_unit) + ' to #' + (S.sel.end_unit === '' ? '?' : S.sel.end_unit)));
      selBar.appendChild(name);
      selBar.appendChild(R.button('Create clip + open editor', function () {
        if (S.sel.start_unit === '' || S.sel.end_unit === '') { R.toast('Pick both start and end', 'bad'); return; }
        return R.createClip({ name: name.value.trim() || 'clip', source_id: id, start_unit: Number(S.sel.start_unit), end_unit: Number(S.sel.end_unit) });
      }, { primary: true, small: true, need: ['clip.create'] }));
    }
    function drawUnits(units) {
      R.clear(unitsBox);
      if (!units.length) { unitsBox.appendChild(h('p', { class: 'muted' }, 'No transcript units. Run "Analyze quick" first (or the transcript is empty).')); return; }
      var box = h('div', { class: 'units' });
      units.forEach(function (u) { box.appendChild(unitRow(u)); });
      unitsBox.appendChild(box);
    }
    function load() {
      return R.run('transcript.get', { source_id: id, format: 'json', limit: 3000 }, true).then(function (d) {
        if (!d) { unitsBox.appendChild(h('p', { class: 'muted' }, 'No transcript yet. Run "Analyze quick".')); return; }
        allUnits = R.extractUnits(d);
        if (!allUnits.length && Array.isArray(d.lines)) R.clear(unitsBox).appendChild(h('pre', null, d.lines.join('\n')));
        else drawUnits(allUnits);
      });
    }
    function search() {
      var term = q.value.trim();
      if (!term) { drawUnits(allUnits); return; }
      return R.run('transcript.search', { source_id: id, query: term, limit: 20 }).then(function (d) { if (d) drawUnits(R.extractUnits(d)); });
    }
    function tiles(u) {
      return R.run('see.tiles', { source_id: id, start: Math.max(0, Number(u.start) - 6), end: Number(u.end) + 6, count: 12 }).then(function (d) {
        if (!d) return;
        R.clear(out);
        out.appendChild(h('div', { class: 'muted small' }, 'Tiles around unit #' + u.id + ' (' + R.fmt(u.start) + ')'));
        out.appendChild(d.image ? R.img(d.image) : h('pre', null, R.pretty(d)));
      });
    }
    q.addEventListener('keydown', function (e) { if (e.key === 'Enter') search(); });
    el.appendChild(h('div', { class: 'row' }, q, R.button('Search', search, { need: ['transcript.search'] }), R.button('Clear', function () { q.value = ''; drawUnits(allUnits); })));
    el.appendChild(selBar);
    el.appendChild(unitsBox);
    el.appendChild(out);
    drawSel();
    load();
  }

  R.createClip = function (body) {
    return R.run('clip.create', body).then(function (d) {
      if (!d) return null;
      var c = d.clip || d;
      var id = R.cid(c);
      R.toast('Clip created', 'good');
      if (id) { R.store('rustler.lastClip', id); R.go('editor', id); }
      return d;
    });
  };

  function highlightsTab(el, s) {
    var id = R.sid(s);
    if (!R.has('highlights.candidates')) {
      el.appendChild(R.na('highlights.candidates'));
      el.appendChild(h('p', { class: 'muted small' }, 'Meanwhile pick units on the Transcript tab, or search for a moment.'));
      return;
    }
    var form = R.schemaForm('highlights.candidates', { fixed: { source_id: id }, submit: 'Find candidates', compact: true, done: draw });
    var list = h('div');
    el.appendChild(form.el);
    el.appendChild(list);
    function draw(d) {
      R.clear(list);
      var items = R.listOf(d, 'candidates', 'windows', 'items');
      if (!items.length) { list.appendChild(h('p', { class: 'muted' }, 'No candidates.')); list.appendChild(R.resultView(d)); return; }
      items.forEach(function (c, i) {
        var su = firstDef(c, ['start_unit', 'unit_start', 'start_unit_id']);
        var eu = firstDef(c, ['end_unit', 'unit_end', 'end_unit_id']);
        var st = firstDef(c, ['start', 'src_start']), en = firstDef(c, ['end', 'src_end']);
        var reasons = c.reasons || c.why || [];
        var text = c.text || c.preview || c.title || (c.opening_text ? c.opening_text + (c.closing_text && c.closing_text !== c.opening_text ? '  …  ' + c.closing_text : '') : '');
        var body = { name: 'Highlight ' + (i + 1), source_id: id };
        if (su !== undefined && eu !== undefined) { body.start_unit = su; body.end_unit = eu; }
        else if (st !== undefined && en !== undefined) { body.start = st; body.end = en; }
        list.appendChild(h('div', { class: 'cand' },
          h('div', { class: 'cand-head' },
            h('strong', null, '#' + (i + 1)),
            c.score !== undefined ? h('span', { class: 'badge agent' }, 'score ' + R.round(c.score, 2)) : null,
            h('span', { class: 'mono small muted' }, (st !== undefined ? R.fmt(st) + ' - ' + R.fmt(en) : '') + (su !== undefined ? '  units #' + su + '-#' + eu : '')),
            h('span', { class: 'grow' }),
            R.button('Create clip', function () { return R.createClip(body); }, { small: true, primary: true, need: ['clip.create'] })),
          text ? h('div', { class: 'cand-text' }, R.short(text, 300)) : null,
          (Array.isArray(reasons) ? reasons : [reasons]).length ? h('div', { class: 'muted small' }, (Array.isArray(reasons) ? reasons : [JSON.stringify(reasons)]).map(function (r) { return typeof r === 'string' ? r : JSON.stringify(r); }).join(' · ')) : null));
      });
    }
  }
  function firstDef(o, keys) { for (var i = 0; i < keys.length; i++) if (o[keys[i]] !== undefined && o[keys[i]] !== null) return o[keys[i]]; return undefined; }

  function understandTab(el, s) {
    var id = R.sid(s);
    var cmds = ['analyze.deep', 'analyze.ocr', 'analyze.events', 'analyze.faces', 'analyze.scenes', 'analyze.audio']
      .filter(R.has);
    if (!cmds.length) { el.appendChild(R.na('analyze.deep', 'analyze.ocr', 'analyze.events', 'analyze.faces')); return; }
    var missingOnes = ['analyze.deep', 'analyze.ocr', 'analyze.events', 'analyze.faces'].filter(function (c) { return !R.has(c); });
    if (missingOnes.length) el.appendChild(h('div', { class: 'muted small' }, 'Not in this build yet: ' + missingOnes.join(', ')));
    // Saved results (written by the analyze.* commands) are shown without re-running.
    var FILES = { 'analyze.deep': 'understanding.json', 'analyze.ocr': 'ocr.json', 'analyze.events': 'events.json', 'analyze.faces': 'faces.json', 'analyze.scenes': 'scenes.json', 'analyze.audio': 'audio_events.json' };
    var top = h('div', { class: 'card inner' }, h('h3', null, 'Summary'), h('div', { class: 'muted small' }, 'loading saved understanding...'));
    el.appendChild(top);
    R.getJson(R.optUrl('analysis/' + id + '/understanding.json')).then(function (u) {
      R.clear(top).appendChild(h('h3', null, 'Summary'));
      if (u && Array.isArray(u.understanding)) top.appendChild(h('ul', { class: 'ulist' }, u.understanding.map(function (l) { return h('li', null, l); })));
      else top.appendChild(h('div', { class: 'muted small' }, R.has('analyze.deep') ? 'No understanding pack yet: run analyze.deep below.' : 'No understanding pack yet.'));
    });
    var grid = h('div', { class: 'ugrid' });
    el.appendChild(grid);
    cmds.forEach(function (c) {
      var out = h('div', { class: 'uout' });
      var spec = R.spec(c);
      if (FILES[c]) R.getJson(R.optUrl('analysis/' + id + '/' + FILES[c])).then(function (d) {
        if (d && !out.firstChild) { out.appendChild(h('div', { class: 'muted small' }, 'saved result (' + FILES[c] + ')')); out.appendChild(summary(d)); }
      });
      grid.appendChild(h('div', { class: 'ucard' },
        h('div', { class: 'row' }, h('strong', { class: 'mono' }, c), h('span', { class: 'grow' }),
          R.button('Run', function () {
            var input = { source_id: id };
            if (spec.annotations && spec.annotations.long_running) {
              return R.jobs.start(c, input).then(function (jid) {
                if (!jid) return;
                R.clear(out).appendChild(h('div', { class: 'muted small' }, 'running...'));
                waitJob(jid, function (d) { R.clear(out).appendChild(summary(d)); });
              });
            }
            return R.run(c, input).then(function (d) { if (d) R.clear(out).appendChild(summary(d)); });
          }, { small: true, primary: true })),
        h('div', { class: 'muted small' }, spec.summary || ''), out));
    });
  }
  // Compact summary: scalar fields as chips, arrays as counts + first rows table.
  function summary(d) {
    var box = h('div');
    if (!d || typeof d !== 'object') { box.appendChild(h('pre', null, R.pretty(d))); return box; }
    if (Array.isArray(d.understanding)) box.appendChild(h('ul', { class: 'ulist' }, d.understanding.map(function (l) { return h('li', null, l); })));
    var chips = h('div', { class: 'chips' });
    Object.keys(d).forEach(function (k) {
      var v = d[k];
      if (k === 'next' || k === 'understanding' || k === 'path' || k === 'source_id') return;
      if (v === null || typeof v === 'object') { if (Array.isArray(v)) chips.appendChild(h('span', { class: 'chipv' }, k + ': ' + v.length)); return; }
      chips.appendChild(h('span', { class: 'chipv' }, k + ': ' + R.short(typeof v === 'number' ? R.round(v, 3) : v, 40)));
    });
    box.appendChild(chips);
    box.appendChild(R.resultView(d));
    return box;
  }

  function infoTab(el, s) {
    var id = R.sid(s);
    if (!R.has('source.show')) { el.appendChild(h('pre', null, R.pretty(s))); return; }
    R.run('source.show', { source_id: id }, true).then(function (d) { el.appendChild(h('pre', null, R.pretty(d || s))); });
  }

  R.views.sources = { label: 'Sources', icon: '▶', render: render };
})();
