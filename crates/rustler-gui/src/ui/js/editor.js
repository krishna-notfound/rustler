// Editor page shell: clip state, revision-checked mutations, undo/redo, keyboard, live sync.
// Panels live in monitor.js (program monitor), regions.js (source frame + region editing),
// timeline.js (tracks) and inspector.js (side tabs). They talk through R.E (state + events).
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  function newState(clipId) {
    return {
      clipId: clipId, data: null, clip: null, rev: 0, timeline: [], duration: 0, fps: 30,
      layout: null, sources: {}, words: {}, units: {}, captions: [], overlays: [], sfx: [], extras: {},
      t: 0, playing: false, selSeg: '', selRegion: '', scopeAll: null, snap: R.store('rustler.snap') !== '0',
      draft: null, redo: [], pending: 0, loading: false, extrasMtime: null, ls: {}
    };
  }
  var E = null;
  R.E = function () { return E; };

  // ------------------------------------------------------------ events
  function on(ev, fn) { (E.ls[ev] = E.ls[ev] || []).push(fn); }
  function emit(ev, arg) { (E.ls[ev] || []).forEach(function (f) { try { f(arg); } catch (e) { console.error(e); } }); }

  // ------------------------------------------------------------ time mapping
  function segAt(t) {
    var tl = E.timeline;
    for (var i = 0; i < tl.length; i++) if (t < tl[i].out_end - 1e-6) return tl[i];
    return tl[tl.length - 1] || null;
  }
  function segById(id) {
    var s = E.clip && E.clip.segments.filter(function (x) { return x.id === id; })[0];
    return s || null;
  }
  function tlById(id) { return E.timeline.filter(function (x) { return x.segment_id === id; })[0] || null; }
  function outToSrc(t) {
    var m = segAt(t);
    if (!m) return null;
    return { seg: m, srcT: R.clamp(m.src_start + (t - m.out_start), m.src_start, m.src_end) };
  }
  // Source time of `sid` -> output time (first segment that contains it) or null.
  function srcToOut(sid, st) {
    for (var i = 0; i < E.timeline.length; i++) {
      var m = E.timeline[i];
      if (m.source_id === sid && st >= m.src_start - 1e-6 && st <= m.src_end + 1e-6) return m.out_start + (st - m.src_start);
    }
    return null;
  }
  function layoutOf(segId) {
    var s = segById(segId);
    return s ? s.layout : null;
  }
  // Same region ids in every segment -> editing "all segments" is meaningful.
  function uniformLayout() {
    if (!E.clip || E.clip.segments.length < 2) return true;
    var key = function (s) { return s.layout.regions.map(function (r) { return r.id; }).sort().join(','); };
    var k0 = key(E.clip.segments[0]);
    return E.clip.segments.every(function (s) { return key(s) === k0; });
  }

  // ------------------------------------------------------------ loading
  function load(reason) {
    if (!E) return Promise.resolve();
    var id = E.clipId;
    E.loading = true;
    var jobs = [R.run('clip.show', { clip_id: id }, true)];
    jobs.push(R.has('layout.show') ? R.run('layout.show', { clip_id: id }, true) : Promise.resolve(null));
    jobs.push(R.has('overlay.list') ? R.run('overlay.list', { clip_id: id }, true) : Promise.resolve(null));
    jobs.push(R.has('audio.list') ? R.run('audio.list', { clip_id: id }, true) : Promise.resolve(null));
    return Promise.all(jobs).then(function (res) {
      E.loading = false;
      if (!E || E.clipId !== id) return;
      var d = res[0];
      if (!d) { emit('error', 'Could not load clip ' + id); return; }
      E.data = d;
      E.clip = d.clip;
      E.rev = d.clip.revision;
      E.timeline = d.timeline || [];
      E.duration = d.duration || 0;
      E.fps = (d.clip.canvas && d.clip.canvas.fps) || 30;
      E.extras = d.extras || {};
      E.layout = res[1];
      E.overlays = res[2] ? R.listOf(res[2], 'overlays', 'items') : [];
      E.audioList = res[3];
      E.sfx = res[3] ? R.listOf(res[3], 'sfx', 'cues', 'items') : [];
      if (E.t > E.duration) E.t = Math.max(0, E.duration - 1 / E.fps);
      if (!segById(E.selSeg)) { var m = segAt(E.t); E.selSeg = m ? m.segment_id : ''; }
      var lay = layoutOf(E.selSeg);
      if (lay && !lay.regions.some(function (r) { return r.id === E.selRegion; })) E.selRegion = lay.regions.length ? lay.regions[lay.regions.length - 1].id : '';
      if (E.scopeAll === null || reason === 'first') E.scopeAll = uniformLayout();
      if (!uniformLayout()) E.scopeAll = false;
      return ensureSources().then(function () {
        emit('load', reason);
        loadCaptions();
      });
    });
  }
  function ensureSources() {
    var need = {};
    E.clip.segments.forEach(function (s) { need[s.source_id] = true; });
    var ids = Object.keys(need).filter(function (sid) { return !E.sources[sid]; });
    if (!ids.length) return Promise.resolve();
    return Promise.all(ids.map(function (sid) {
      var p1 = R.has('source.show') ? R.run('source.show', { source_id: sid }, true) : Promise.resolve(null);
      return p1.then(function (d) {
        var src = d ? (d.source || d) : { id: sid, probe: {} };
        E.sources[sid] = src;
        // Words for snapping + captions lane: the transcript file itself (read-only).
        return R.getJson(R.fileUrl('analysis/' + sid + '/transcript.json')).then(function (tr) {
          E.words[sid] = tr && Array.isArray(tr.words) ? tr.words : [];
        });
      });
    }));
  }
  // Caption cues in output time, from caption.export_srt (the real chunking of the renderer).
  function loadCaptions() {
    if (!E.clip.captions || !E.clip.captions.enabled || !R.has('caption.export_srt')) { E.captions = []; emit('captions'); return; }
    var id = E.clipId;
    R.run('caption.export_srt', { clip_id: id }, true).then(function (d) {
      if (!d || !E || E.clipId !== id) return;
      return fetch(R.fileUrl(d.path), { headers: { 'x-rustler-token': R.T } }).then(function (r) { return r.ok ? r.text() : ''; }).then(function (txt) {
        E.captions = parseSrt(txt);
        emit('captions');
      });
    });
  }
  function parseSrt(txt) {
    var out = [];
    String(txt || '').replace(/\r/g, '').split(/\n\n+/).forEach(function (block) {
      var lines = block.split('\n').filter(Boolean);
      var i = lines.findIndex(function (l) { return l.indexOf('-->') >= 0; });
      if (i < 0) return;
      var m = lines[i].split('-->').map(function (x) { return x.trim(); });
      out.push({ start: srtT(m[0]), end: srtT(m[1]), text: lines.slice(i + 1).join(' ') });
    });
    return out;
  }
  function srtT(s) {
    var m = /(\d+):(\d+):(\d+)[,.](\d+)/.exec(s || '');
    return m ? (+m[1]) * 3600 + (+m[2]) * 60 + (+m[3]) + (+m[4]) / 1000 : 0;
  }

  // ------------------------------------------------------------ mutations
  // Runs a clip-changing command with the right revision guard; reloads afterwards.
  function mutate(cmd, input, opts) {
    var o = opts || {};
    if (!R.has(cmd)) { R.toast(cmd + ' is not available in this build', 'bad'); return Promise.resolve(null); }
    var body = Object.assign({}, input);
    if (R.hasProp(cmd, 'clip_id') && body.clip_id === undefined) body.clip_id = E.clipId;
    if (R.hasProp(cmd, 'base_revision') && body.base_revision === undefined) body.base_revision = E.rev;
    if (R.hasProp(cmd, 'expect_revision') && body.expect_revision === undefined) body.expect_revision = E.rev;
    E.pending++;
    return R.call(cmd, body).then(function (env) {
      E.pending--;
      if (env && env.ok) {
        if (!o.keepRedo) E.redo = [];
        (env.warnings || []).forEach(function (w) { R.toast('warning: ' + w); });
        (env.data && env.data.warnings || []).forEach(function (w) { R.toast(w); });
        if (o.toast) R.toast(o.toast, 'good');
        return load('mutate').then(function () { return env.data; });
      }
      var code = env && env.error && env.error.code;
      if (code === 'revision_conflict') {
        R.toast('The clip changed elsewhere; reloaded the latest version. Try again.', 'bad');
        return load('conflict').then(function () { return null; });
      }
      R.toast(R.errText(cmd, env), 'bad');
      emit('load', 'failed');   // redraw drafts back to the saved state
      return null;
    });
  }
  function undo() {
    if (!R.has('history.undo')) return;
    var from = E.rev;
    return mutate('history.undo', {}, { keepRedo: true }).then(function (d) { if (d) { E.redo.push(from); R.toast('Undone (revision ' + from + ' kept for redo)'); } });
  }
  function redo() {
    if (!E.redo.length || !R.has('history.restore')) { R.toast('Nothing to redo'); return; }
    var rev = E.redo.pop();
    return mutate('history.restore', { revision: rev }, { keepRedo: true }).then(function (d) { if (d) R.toast('Redone (restored revision ' + rev + ')'); else E.redo = []; });
  }

  // ------------------------------------------------------------ live sync
  function syncTick() {
    if (!E || E.pending || E.loading) return;
    R.getJson(R.apiUrl('/api/clip_state', { clip_id: E.clipId })).then(function (st) {
      if (!st || !E || E.pending || E.loading || st.clip_id !== E.clipId) return;
      var extrasChanged = E.extrasMtime !== null && st.extras_mtime_ms !== E.extrasMtime;
      var first = E.extrasMtime === null;
      E.extrasMtime = st.extras_mtime_ms;
      if ((st.revision !== null && st.revision !== E.rev) || (extrasChanged && !first)) {
        load('sync').then(function () { R.toast('Clip updated by agent (revision ' + E.rev + ')', 'agent'); });
      }
    });
  }

  // ------------------------------------------------------------ keyboard
  function onKey(e) {
    if (!E || S.view !== 'editor') return;
    var tag = (e.target && e.target.tagName) || '';
    if (/INPUT|TEXTAREA|SELECT/.test(tag) || (e.target && e.target.isContentEditable)) return;
    if (document.querySelector('.modal-back')) return;
    var k = e.key;
    var ctrl = e.ctrlKey || e.metaKey;
    var frame = 1 / E.fps;
    if (k === ' ') { e.preventDefault(); R.monitor.toggle(); }
    else if (ctrl && (k === 'z' || k === 'Z') && !e.shiftKey) { e.preventDefault(); undo(); }
    else if (ctrl && ((k === 'z' || k === 'Z') && e.shiftKey || k === 'y' || k === 'Y')) { e.preventDefault(); redo(); }
    else if (!ctrl && (k === 's' || k === 'S')) { e.preventDefault(); split(); }
    else if (k === 'Delete' || k === 'Backspace') { e.preventDefault(); deleteSeg(); }
    else if (k === 'ArrowLeft') { e.preventDefault(); R.monitor.seek(E.t - (e.shiftKey ? 1 : frame)); }
    else if (k === 'ArrowRight') { e.preventDefault(); R.monitor.seek(E.t + (e.shiftKey ? 1 : frame)); }
    else if (k === 'Home') { e.preventDefault(); R.monitor.seek(0); }
    else if (k === 'End') { e.preventDefault(); R.monitor.seek(E.duration - frame); }
    else if (k === 'j' || k === 'J') { R.monitor.seek(E.t - 2); }
    else if (k === 'l' || k === 'L') { R.monitor.toggle(); }
  }
  function split() {
    var m = segAt(E.t);
    if (!m) return;
    if (E.t - m.out_start < 0.1 || m.out_end - E.t < 0.1) { R.toast('Move the playhead inside a segment (not at its edge) to split'); return; }
    return mutate('edit.split', { out_t: R.round(E.t, 3) }, { toast: 'Split at ' + R.tc(E.t, E.fps) });
  }
  function deleteSeg() {
    if (!E.selSeg) return;
    if (E.clip.segments.length < 2) { R.toast('A clip needs at least one segment', 'bad'); return; }
    return mutate('edit.delete_segment', { segment_id: E.selSeg }, { toast: 'Segment deleted (Ctrl+Z to undo)' });
  }
  function selectSeg(id, seek) {
    E.selSeg = id;
    var lay = layoutOf(id);
    if (lay && !lay.regions.some(function (r) { return r.id === E.selRegion; })) E.selRegion = lay.regions.length ? lay.regions[lay.regions.length - 1].id : '';
    if (seek) { var m = tlById(id); if (m && (E.t < m.out_start || E.t >= m.out_end)) R.monitor.seek(m.out_start + 0.001); }
    emit('select');
  }
  function selectRegion(id) { E.selRegion = id; emit('select'); }

  // ------------------------------------------------------------ page
  function render(v, clipId) {
    if (!clipId) { return picker(v); }
    if (!R.has('clip.show')) { v.appendChild(R.na('clip.show')); return; }
    R.store('rustler.lastClip', clipId);
    E = newState(clipId);
    E.on = on; E.emit = emit; E.mutate = mutate; E.reload = load; E.segAt = segAt; E.segById = segById; E.tlById = tlById;
    E.outToSrc = outToSrc; E.srcToOut = srcToOut; E.layoutOf = layoutOf; E.undo = undo; E.redo = redo; E.split = split;
    E.deleteSeg = deleteSeg; E.selectSeg = selectSeg; E.selectRegion = selectRegion; E.uniformLayout = uniformLayout;

    var name = h('div', { class: 'ed-name' }, 'loading...');
    var revChip = h('span', { class: 'chip small' }, '');
    var tb = h('div', { class: 'ed-toolbar' },
      R.button('← Clips', function () { R.go('clips'); }, { small: true, cls: 'ghost' }),
      name, revChip,
      h('span', { class: 'grow' }),
      R.button('↶ Undo', undo, { small: true, need: ['history.undo'], title: 'Ctrl+Z' }),
      R.button('↷ Redo', redo, { small: true, need: ['history.restore'], title: 'Ctrl+Shift+Z' }),
      R.button('Split (S)', split, { small: true, need: ['edit.split'] }),
      R.button('Render draft', function () { return R.renderClip('draft'); }, { small: true, need: ['render.run'] }),
      R.button('Render final', function () { return R.renderClip('final'); }, { small: true, primary: true, need: ['render.run'] }),
      R.button('?', function () { help(); }, { small: true, cls: 'ghost', title: 'Keyboard shortcuts' }));
    var srcPanel = h('div', { class: 'ed-panel ed-source' });
    var progPanel = h('div', { class: 'ed-panel ed-program' });
    var insp = h('div', { class: 'ed-panel ed-inspector' });
    var tl = h('div', { class: 'ed-timeline' });
    var root = h('div', { class: 'editor' }, tb, h('div', { class: 'ed-main' }, srcPanel, progPanel, insp), tl);
    v.appendChild(root);

    R.monitor.mount(progPanel, E);
    R.regions.mount(srcPanel, E);
    R.timeline.mount(tl, E);
    R.inspector.mount(insp, E);

    on('load', function () {
      name.textContent = E.clip.name;
      revChip.textContent = 'rev ' + E.rev + '  ·  ' + R.tc(E.duration, E.fps) + '  ·  ' + E.clip.segments.length + ' seg';
    });
    on('error', function (msg) { name.textContent = msg; name.classList.add('err'); });

    document.addEventListener('keydown', onKey);
    R.onLeave(function () { document.removeEventListener('keydown', onKey); R.monitor.unmount(); E = null; });
    R.every(1500, syncTick);
    load('first');
  }
  function help() {
    var rows = [['Space / L', 'play / pause'], ['S', 'split at playhead'], ['Delete', 'delete selected segment'], ['Ctrl+Z', 'undo'], ['Ctrl+Shift+Z / Ctrl+Y', 'redo'],
      ['← / →', '1 frame'], ['Shift+← / →', '1 second'], ['Home / End', 'start / end'], ['J', 'back 2 s'],
      ['Drag box', 'move a region (snaps to edges; hold Alt to disable)'], ['Drag corner/edge', 'resize a region'], ['Drag segment edge', 'trim (word-snapped when Snap is on)'], ['Drag segment', 'reorder']];
    R.modal('Keyboard and mouse', h('table', { class: 'tbl' }, h('tbody', null, rows.map(function (r) { return h('tr', null, h('td', { class: 'mono' }, r[0]), h('td', null, r[1])); }))));
  }
  function picker(v) {
    v.appendChild(h('h1', null, 'Editor'));
    v.appendChild(h('p', { class: 'muted' }, 'Pick a clip to edit.'));
    var list = h('div', { class: 'card' });
    v.appendChild(list);
    if (!R.has('clip.list')) { list.appendChild(R.na('clip.list')); return; }
    R.run('clip.list', {}).then(function (d) {
      var items = R.listOf(d, 'clips');
      if (!items.length) { list.appendChild(h('p', { class: 'muted' }, 'No clips yet. Create one on the Clips or Sources page.')); return; }
      items.forEach(function (c) {
        list.appendChild(h('div', { class: 'row' }, h('strong', null, c.name || R.cid(c)), h('span', { class: 'muted small' }, c.summary || ''), h('span', { class: 'grow' }),
          R.button('Open', function () { R.go('editor', R.cid(c)); }, { primary: true, small: true })));
      });
    });
  }

  // Render via the job queue (progress shown in the Render tab + top chip).
  R.renderClip = function (quality) {
    if (!E) return;
    var id = E.clipId;
    return R.jobs.start('render.run', { clip_id: id, quality: quality }).then(function (jid) {
      if (!jid) return;
      R.toast('Rendering ' + quality + ' in the background...');
      if (R.inspector && R.inspector.showTab) R.inspector.showTab('render');
      var off = R.jobs.on(function (list) {
        var j = list.filter(function (x) { return x.id === jid; })[0];
        if (!j || j.status === 'running') return;
        off();
        if (E && E.clipId === id) load('render');
      });
    });
  };

  R.views.editor = { label: 'Editor', icon: '▦', render: render };
})();
