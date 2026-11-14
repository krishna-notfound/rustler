// Timeline: ruler + tracks (V segments, captions, overlays, audio). Zoom/scroll, playhead,
// word-snapped trims (edit.trim), split (edit.split), delete, drag-reorder (edit.move_segment),
// overlay retime (overlay.edit when registered).
(function () {
  'use strict';
  var R = window.R, h = R.h;
  var TL = R.timeline = {};
  var E = null, scroll = null, inner = null, ruler = null, playhead = null, tracks = {}, zoom = 60, heads = null, zoomIn = null, snapChk = null, tcEl = null;

  // ------------------------------------------------------------ overlay helpers (schema-agnostic)
  var START_KEYS = ['start', 'out_start', 'start_t', 'from'], END_KEYS = ['end', 'out_end', 'end_t', 'to'];
  function pick(o, keys) { for (var i = 0; i < keys.length; i++) if (typeof o[keys[i]] === 'number') return keys[i]; return null; }
  R.overlayTimes = function (o) {
    var sk = pick(o, START_KEYS), ek = pick(o, END_KEYS);
    if (sk && ek) return { start: o[sk], end: o[ek] };
    if (sk && typeof o.duration === 'number') return { start: o[sk], end: o[sk] + o.duration };
    var st = R.E && R.E();
    if (sk && st) return { start: o[sk], end: st.duration };            // open-ended: until the clip end
    if (!sk && ek && st) return { start: 0, end: o[ek] };
    if (!sk && !ek && st && ('start' in o || 'end' in o)) return { start: 0, end: st.duration };
    return null;
  };
  R.overlayId = function (o) { return o.id || o.overlay_id || o.name; };
  // Build an overlay.edit input that retimes `o` (keys taken from the command schema).
  R.overlayRetime = function (o, start, end) {
    var props = R.props('overlay.edit');
    var idKey = ['overlay_id', 'id'].filter(function (k) { return props[k]; })[0] || 'overlay_id';
    var sk = START_KEYS.filter(function (k) { return props[k]; })[0];
    var ek = END_KEYS.filter(function (k) { return props[k]; })[0];
    if (!sk || !ek) return null;
    var input = {}; input[idKey] = R.overlayId(o); input[sk] = R.round(start, 3); input[ek] = R.round(end, 3);
    return input;
  };

  // ------------------------------------------------------------ mount
  TL.mount = function (host, state) {
    E = state;
    zoom = Number(R.store('rustler.zoom')) || 60;
    zoomIn = h('input', { type: 'range', min: 8, max: 400, step: 1, value: zoom, title: 'zoom (Ctrl+wheel)' });
    zoomIn.addEventListener('input', function () { autoFit = false; setZoom(Number(zoomIn.value), true); });
    snapChk = h('input', { type: 'checkbox', checked: E.snap });
    snapChk.addEventListener('change', function () { E.snap = snapChk.checked; R.store('rustler.snap', E.snap ? '1' : '0'); draw(); });
    tcEl = h('span', { class: 'tc' }, '00:00.00');
    host.appendChild(h('div', { class: 'tl-bar' },
      h('strong', null, 'Timeline'), tcEl,
      R.button('Split', function () { return E.split(); }, { small: true, need: ['edit.split'], title: 'S' }),
      R.button('Delete seg', function () { return E.deleteSeg(); }, { small: true, need: ['edit.delete_segment'], title: 'Delete' }),
      h('label', { class: 'small chk', title: 'snap trims and the playhead to transcript words' }, snapChk, 'snap to words'),
      h('span', { class: 'grow' }),
      h('span', { class: 'muted small' }, 'zoom'), zoomIn,
      R.button('Fit', function () { fit(); }, { small: true })));
    heads = h('div', { class: 'tl-heads' });
    scroll = h('div', { class: 'tl-scroll' });
    inner = h('div', { class: 'tl-inner' });
    ruler = h('div', { class: 'tl-ruler' });
    playhead = h('div', { class: 'tl-playhead' }, h('i'));
    inner.appendChild(ruler);
    [['v', 'V1', 'video segments'], ['cap', 'CAP', 'caption chunks'], ['txt', 'TXT', 'text overlays'], ['a', 'A1', 'audio: source, music bed, sfx']].forEach(function (t) {
      tracks[t[0]] = h('div', { class: 'track tr-' + t[0] });
      inner.appendChild(tracks[t[0]]);
      heads.appendChild(h('div', { class: 'thead th-' + t[0], title: t[2] }, t[1]));
    });
    inner.appendChild(playhead);
    scroll.appendChild(inner);
    host.appendChild(h('div', { class: 'tl-body' }, heads, scroll));

    // scrub on the ruler / empty track area
    ruler.addEventListener('pointerdown', scrubStart);
    Object.keys(tracks).forEach(function (k) { tracks[k].addEventListener('pointerdown', function (e) { if (e.target === tracks[k]) scrubStart(e); }); });
    scroll.addEventListener('wheel', function (e) {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      var b = scroll.getBoundingClientRect();
      var at = (e.clientX - b.left + scroll.scrollLeft) / zoom;
      autoFit = false;
      setZoom(zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15));
      scroll.scrollLeft = at * zoom - (e.clientX - b.left);
    }, { passive: false });
    E.on('load', function (reason) { if (reason === 'first' || autoFit) fit(); else draw(); });
    var onResize = function () { if (autoFit) fit(); else draw(); };
    window.addEventListener('resize', onResize);
    R.onLeave(function () { window.removeEventListener('resize', onResize); });
    E.on('captions', drawCaptions);
    E.on('select', draw);
    E.on('time', movePlayhead);
  };

  function setZoom(z, keepCenter) {
    var center = scroll ? (scroll.scrollLeft + scroll.clientWidth / 2) / zoom : 0;
    zoom = R.clamp(z, 4, 600);
    zoomIn.value = Math.round(R.clamp(zoom, 8, 400));
    R.store('rustler.zoom', String(Math.round(zoom)));
    draw();
    if (keepCenter) scroll.scrollLeft = center * zoom - scroll.clientWidth / 2;
  }
  var autoFit = true;   // follow the window size until the user zooms
  function fit() {
    if (!E || !E.duration) return;
    setZoom((scroll.clientWidth - 40) / E.duration);
    autoFit = true;
    scroll.scrollLeft = 0;
  }
  function X(t) { return t * zoom; }

  function scrubStart(e) {
    if (e.button !== 0 || !E) return;
    e.preventDefault();
    var b = inner.getBoundingClientRect();
    var go = function (ev) {
      var t = (ev.clientX - b.left) / zoom;
      if (E.snap && !ev.altKey) t = snapOut(t, 6 / zoom);
      if (E.playing) R.monitor.pause();
      R.monitor.seek(t);
    };
    go(e);
    var el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    var mv = function (ev) { go(ev); };
    var up = function () { el.removeEventListener('pointermove', mv); el.removeEventListener('pointerup', up); };
    el.addEventListener('pointermove', mv); el.addEventListener('pointerup', up);
  }

  // ------------------------------------------------------------ word snapping
  // Words of the clip in output time: [{t0, t1, text}]
  var owCache = { key: '', list: [] };
  function outWords() {
    var key = E.clipId + '@' + E.rev + '@' + Object.keys(E.words).map(function (k) { return E.words[k].length; }).join(',');
    if (owCache.key === key) return owCache.list;
    var out = [];
    owCache = { key: key, list: out };
    E.timeline.forEach(function (m) {
      (E.words[m.source_id] || []).forEach(function (w) {
        if (w.end <= m.src_start || w.start >= m.src_end) return;
        out.push({ t0: m.out_start + Math.max(0, w.start - m.src_start), t1: m.out_start + Math.min(m.src_end, w.end) - m.src_start, text: w.text });
      });
    });
    return out;
  }
  function snapOut(t, tol) {
    var best = t, bd = tol;
    outWords().forEach(function (w) { [w.t0, w.t1].forEach(function (x) { var d = Math.abs(x - t); if (d < bd) { bd = d; best = x; } }); });
    E.timeline.forEach(function (m) { [m.out_start, m.out_end].forEach(function (x) { var d = Math.abs(x - t); if (d < bd) { bd = d; best = x; } }); });
    return best;
  }
  // Nearest word boundary in SOURCE time (kind: 'start' | 'end').
  function snapSrc(sid, st, kind, tol) {
    var best = st, bd = tol;
    (E.words[sid] || []).forEach(function (w) { var x = kind === 'start' ? w.start : w.end; var d = Math.abs(x - st); if (d < bd) { bd = d; best = x; } });
    return best;
  }

  // ------------------------------------------------------------ drawing
  function draw() {
    if (!E || !E.clip) return;
    var W = Math.max(scroll.clientWidth, X(E.duration) + 160);
    inner.style.width = W + 'px';
    drawRuler(W);
    drawSegments();
    drawCaptions();
    drawOverlays();
    drawAudio();
    movePlayhead();
  }
  function drawRuler(W) {
    R.clear(ruler);
    var steps = [1 / E.fps, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
    var step = steps.filter(function (s) { return s * zoom >= 64; })[0] || 600;
    var minor = step / 5;
    for (var t = 0; t * zoom < W; t += minor) {
      var major = Math.abs(t / step - Math.round(t / step)) < 1e-6;
      ruler.appendChild(h('i', { class: major ? 'mj' : 'mn', style: { left: X(t) + 'px' } }));
      if (major) ruler.appendChild(h('span', { style: { left: X(t) + 3 + 'px' } }, R.tc(t, E.fps)));
    }
    ruler.appendChild(h('div', { class: 'tl-end', style: { left: X(E.duration) + 'px' }, title: 'clip end' }));
  }
  function drawSegments() {
    var tr = R.clear(tracks.v);
    var words = E.snap ? outWords() : [];
    E.timeline.forEach(function (m, i) {
      var seg = E.segById(m.segment_id);
      var src = E.sources[m.source_id] || {};
      var el = h('div', { class: 'seg' + (m.segment_id === E.selSeg ? ' sel' : ''), style: { left: X(m.out_start) + 'px', width: Math.max(4, X(m.out_end - m.out_start)) + 'px' }, title: 'drag to reorder · drag edges to trim' },
        h('i', { class: 'trim l', 'data-e': 'l' }), h('i', { class: 'trim r', 'data-e': 'r' }),
        h('div', { class: 'seg-l' }, h('b', null, (i + 1) + ' '), (seg && seg.layout.preset) || 'custom', ' · ', src.name || m.source_id),
        h('div', { class: 'seg-s mono' }, R.fmt(m.src_start) + '–' + R.fmt(m.src_end)));
      if (words.length) {
        var wl = h('div', { class: 'wticks' });
        words.forEach(function (w) { if (w.t0 >= m.out_start && w.t0 < m.out_end) wl.appendChild(h('i', { style: { left: X(w.t0 - m.out_start) + 'px' } })); });
        el.appendChild(wl);
      }
      el.addEventListener('pointerdown', function (e) { segDown(e, m, el); });
      tr.appendChild(el);
    });
  }
  function drawCaptions() {
    if (!tracks.cap) return;
    var tr = R.clear(tracks.cap);
    if (!E.clip.captions.enabled) { tr.appendChild(h('span', { class: 'tnote' }, 'captions off')); return; }
    if (!E.captions.length) { tr.appendChild(h('span', { class: 'tnote' }, R.has('caption.export_srt') ? 'no caption words (analyze the source first)' : 'caption.export_srt not available')); return; }
    E.captions.forEach(function (c) {
      var el = h('div', { class: 'cue', style: { left: X(c.start) + 'px', width: Math.max(2, X(c.end - c.start) - 1) + 'px' }, title: R.tc(c.start, E.fps) + '  ' + c.text }, c.text);
      el.addEventListener('pointerdown', function (e) { e.stopPropagation(); R.monitor.seek(c.start + 0.001); });
      tr.appendChild(el);
    });
  }
  function drawOverlays() {
    var tr = R.clear(tracks.txt);
    if (!R.has('overlay.list')) { tr.appendChild(h('span', { class: 'tnote' }, 'text overlays: overlay.* not in this build')); return; }
    if (!E.overlays.length) { tr.appendChild(h('span', { class: 'tnote' }, 'no text overlays — add one in the Overlays tab')); return; }
    E.overlays.forEach(function (o) {
      var tm = R.overlayTimes(o);
      if (!tm) return;
      var el = h('div', { class: 'ovl', style: { left: X(tm.start) + 'px', width: Math.max(6, X(tm.end - tm.start)) + 'px' }, title: (o.text || '') + ' (drag to retime)' },
        h('i', { class: 'trim l', 'data-e': 'l' }), h('i', { class: 'trim r', 'data-e': 'r' }), h('span', null, o.text || R.overlayId(o) || 'overlay'));
      el.addEventListener('pointerdown', function (e) { ovlDown(e, o, tm, el); });
      tr.appendChild(el);
    });
  }
  function drawAudio() {
    var tr = R.clear(tracks.a);
    E.timeline.forEach(function (m) {
      var src = E.sources[m.source_id] || {};
      if (src.probe && src.probe.has_audio === false) return;
      tr.appendChild(h('div', { class: 'aud', style: { left: X(m.out_start) + 'px', width: Math.max(2, X(m.out_end - m.out_start) - 1) + 'px' } }, h('span', null, 'source audio')));
    });
    var mu = E.extras && E.extras.music;
    if (mu) {
      var name = String(mu.path || '').split(/[\\/]/).pop();
      tr.appendChild(h('div', { class: 'music', style: { left: '0px', width: X(E.duration) + 'px' }, title: R.pretty(mu) },
        '♫ ' + name + '  ' + (mu.volume_db !== undefined ? mu.volume_db + ' dB' : '') + (mu.duck ? '  · ducked' : '')));
    }
    (E.sfx || []).forEach(function (s) {
      var t = typeof s.at_out_t === 'number' ? s.at_out_t : (typeof s.out_t === 'number' ? s.out_t : (typeof s.at === 'number' ? s.at : (typeof s.start === 'number' ? s.start : null)));
      if (t === null) return;
      tr.appendChild(h('div', { class: 'sfx', style: { left: X(t) + 'px' }, title: R.pretty(s) }, '• ' + String(s.name || s.file || s.asset || 'sfx').split(/[\\/]/).pop()));
    });
  }
  function movePlayhead() {
    if (!E || !playhead) return;
    var x = X(E.t);
    playhead.style.transform = 'translateX(' + x + 'px)';
    tcEl.textContent = R.tc(E.t, E.fps) + ' / ' + R.tc(E.duration, E.fps);
    if (E.playing) {
      if (x > scroll.scrollLeft + scroll.clientWidth - 40 || x < scroll.scrollLeft) scroll.scrollLeft = x - 60;
    }
  }

  // ------------------------------------------------------------ segment interaction
  function segDown(e, m, el) {
    if (e.button !== 0) return;
    e.preventDefault(); e.stopPropagation();
    if (E.playing) R.monitor.pause();
    var edge = e.target.getAttribute('data-e');
    var b = inner.getBoundingClientRect();
    var x0 = e.clientX;
    var moved = false;
    el.setPointerCapture(e.pointerId);
    var left0 = X(m.out_start), w0 = X(m.out_end - m.out_start);
    var srcDur = ((E.sources[m.source_id] || {}).probe || {}).duration || Infinity;
    var ghost = null, tip = h('div', { class: 'tl-tip' });
    var result = null;
    function mv(ev) {
      var dx = ev.clientX - x0;
      if (!moved && Math.abs(dx) < 3) return;
      if (!moved) { moved = true; inner.appendChild(tip); }
      var dt = dx / zoom;
      if (edge === 'l') {
        var ns = R.clamp(m.src_start + dt, 0, m.src_end - 0.1);
        if (E.snap && !ev.altKey) ns = snapSrc(m.source_id, ns, 'start', 8 / zoom);
        result = { start: ns };
        var d = ns - m.src_start;
        el.style.left = left0 + 'px'; el.style.width = Math.max(4, w0 - X(d)) + 'px';
        tip.textContent = 'start ' + R.fmt(ns) + ' (' + (d >= 0 ? '+' : '') + d.toFixed(2) + ' s)';
        tip.style.left = left0 + 'px';
      } else if (edge === 'r') {
        var ne = R.clamp(m.src_end + dt, m.src_start + 0.1, srcDur);
        if (E.snap && !ev.altKey) ne = snapSrc(m.source_id, ne, 'end', 8 / zoom);
        result = { end: ne };
        var d2 = ne - m.src_end;
        el.style.width = Math.max(4, w0 + X(d2)) + 'px';
        tip.textContent = 'end ' + R.fmt(ne) + ' (' + (d2 >= 0 ? '+' : '') + d2.toFixed(2) + ' s)';
        tip.style.left = left0 + w0 + X(d2) + 'px';
      } else {
        if (E.timeline.length < 2) return;
        el.style.transform = 'translateX(' + dx + 'px)';
        el.classList.add('dragging');
        var cx = ev.clientX - b.left;
        var idx = 0;
        E.timeline.forEach(function (o) { if (o !== m && X((o.out_start + o.out_end) / 2) < cx) idx++; });
        result = { to_index: idx };
        if (!ghost) { ghost = h('div', { class: 'tl-drop' }); tracks.v.appendChild(ghost); }
        var others = E.timeline.filter(function (o) { return o !== m; });
        var gx = idx < others.length ? X(others[idx].out_start) : X(others[others.length - 1].out_end);
        ghost.style.left = gx + 'px';
        tip.textContent = 'move to position ' + (idx + 1);
        tip.style.left = cx + 'px';
      }
    }
    function up(ev) {
      el.removeEventListener('pointermove', mv); el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up);
      if (tip.parentNode) tip.parentNode.removeChild(tip);
      if (!moved) {
        E.selectSeg(m.segment_id);
        var t = (ev.clientX - b.left) / zoom;
        if (E.snap && !ev.altKey) t = snapOut(t, 6 / zoom);
        R.monitor.seek(t);
        return;
      }
      if (!result) { draw(); return; }
      if (result.to_index !== undefined) {
        var cur = E.timeline.indexOf(m);
        if (result.to_index === cur) { draw(); return; }
        E.mutate('edit.move_segment', { segment_id: m.segment_id, to_index: result.to_index }, { toast: 'Segment moved' });
      } else {
        var input = { segment_id: m.segment_id };
        if (result.start !== undefined) input.start = R.round(result.start, 3);
        if (result.end !== undefined) input.end = R.round(result.end, 3);
        if (E.snap) input.snap = 'word';
        E.mutate('edit.trim', input, { toast: 'Trimmed' });
      }
    }
    el.addEventListener('pointermove', mv); el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
  }

  // ------------------------------------------------------------ overlay interaction
  function ovlDown(e, o, tm, el) {
    if (e.button !== 0) return;
    e.preventDefault(); e.stopPropagation();
    var edge = e.target.getAttribute('data-e');
    var x0 = e.clientX, moved = false, ns = tm.start, ne = tm.end;
    el.setPointerCapture(e.pointerId);
    function mv(ev) {
      var dx = ev.clientX - x0;
      if (!moved && Math.abs(dx) < 3) return;
      moved = true;
      var dt = dx / zoom;
      if (edge === 'l') ns = R.clamp(tm.start + dt, 0, tm.end - 0.1);
      else if (edge === 'r') ne = R.clamp(tm.end + dt, tm.start + 0.1, E.duration);
      else { var len = tm.end - tm.start; ns = R.clamp(tm.start + dt, 0, Math.max(0, E.duration - len)); ne = ns + len; }
      if (E.snap && !ev.altKey) { var s2 = snapOut(ns, 6 / zoom); if (edge !== 'r') { var sh = s2 - ns; ns = s2; if (!edge) ne += sh; } }
      el.style.left = X(ns) + 'px'; el.style.width = Math.max(6, X(ne - ns)) + 'px';
    }
    function up() {
      el.removeEventListener('pointermove', mv); el.removeEventListener('pointerup', up);
      if (!moved) { R.monitor.seek(tm.start + 0.001); if (R.inspector) R.inspector.showTab('overlays'); return; }
      var input = R.overlayRetime(o, ns, ne);
      if (!input || !R.has('overlay.edit')) { R.toast('overlay.edit cannot retime in this build', 'bad'); draw(); return; }
      E.mutate('overlay.edit', input, { toast: 'Overlay retimed' });
    }
    el.addEventListener('pointermove', mv); el.addEventListener('pointerup', up);
  }

  TL.draw = function () { draw(); };
  TL.outWords = function () { return E ? outWords() : []; };
})();
