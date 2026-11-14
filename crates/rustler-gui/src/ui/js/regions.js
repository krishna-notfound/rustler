// Region editor: the Source panel (source frame at the playhead with each region's src rect) and
// the dst rects drawn over the Program canvas. Boxes drag/resize with snapping; on release ->
// layout.edit_region (revision-checked). Also: add region, detect webcam, streamer split.
(function () {
  'use strict';
  var R = window.R, h = R.h;
  var G = R.regions = {};
  var E = null, srcLayer = null, dstLayerObj = null, srcStage = null, mediaBox = null, info = null, segSel = null, scopeChk = null, suggestLayer = null, bar = null;
  var COLORS = ['#ff8a3d', '#3dd6ff', '#b18cff', '#5be37d', '#ff5c8a', '#ffd23d'];
  var MIN = 0.02, SNAP = 0.012;

  G.color = function (ix) { return COLORS[ix % COLORS.length]; };

  // Fit `box` (aspect ar = w/h) inside `stage`, keeping aspect.
  R.fitBox = function (stage, box, arFn) {
    function fit() {
      var ar = arFn();
      var sw = stage.clientWidth, sh = stage.clientHeight;
      if (!sw || !sh || !ar) return;
      var w = Math.min(sw, sh * ar), hh = w / ar;
      box.style.width = Math.floor(w) + 'px'; box.style.height = Math.floor(hh) + 'px';
    }
    var ro = new ResizeObserver(fit);
    ro.observe(stage);
    R.onLeave(function () { ro.disconnect(); });
    fit();
    return fit;
  };

  function clampRect(r) {
    var w = R.clamp(r.w, MIN, 1), hh = R.clamp(r.h, MIN, 1);
    return { x: R.round(R.clamp(r.x, 0, 1 - w)), y: R.round(R.clamp(r.y, 0, 1 - hh)), w: R.round(w), h: R.round(hh) };
  }
  // Snap value v to the nearest target within SNAP; returns [snapped, diff].
  function snapTo(v, targets) {
    var best = v, bd = SNAP;
    targets.forEach(function (t) { var d = Math.abs(v - t); if (d < bd) { bd = d; best = t; } });
    return best;
  }

  // ------------------------------------------------------------ a layer of draggable rects
  function RectLayer(el, which) {
    this.el = el; this.which = which; this.boxes = {}; this.drag = null; this.guides = h('div', { class: 'guides' });
    el.appendChild(this.guides);
  }
  RectLayer.prototype.update = function (regs, selId) {
    var self = this, seen = {};
    regs.forEach(function (r) {
      seen[r.id] = true;
      var b = self.boxes[r.id];
      if (!b) { b = self.boxes[r.id] = self.makeBox(r.id); self.el.appendChild(b); }
      var rect = r[self.which];
      if (!(self.drag && self.drag.id === r.id)) {
        b.style.left = rect.x * 100 + '%'; b.style.top = rect.y * 100 + '%';
        b.style.width = rect.w * 100 + '%'; b.style.height = rect.h * 100 + '%';
      }
      b.style.setProperty('--rc', G.color(r.ix));
      b.style.zIndex = 10 + (r.z || 0) + (r.id === selId ? 100 : 0);
      b.classList.toggle('sel', r.id === selId);
      b.querySelector('.rlabel').textContent = r.id + (self.which === 'dst' ? ' · ' + r.fit : '');
    });
    Object.keys(self.boxes).forEach(function (id) { if (!seen[id]) { self.el.removeChild(self.boxes[id]); delete self.boxes[id]; } });
  };
  RectLayer.prototype.makeBox = function (id) {
    var self = this;
    var b = h('div', { class: 'rbox ' + self.which, 'data-id': id }, h('span', { class: 'rlabel' }, id));
    ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'].forEach(function (k) { b.appendChild(h('i', { class: 'hd ' + k, 'data-h': k })); });
    b.addEventListener('pointerdown', function (e) { self.down(e, id, e.target.getAttribute('data-h') || 'move'); });
    return b;
  };
  RectLayer.prototype.down = function (e, id, mode) {
    if (e.button !== 0 || !E || !E.clip) return;
    e.preventDefault(); e.stopPropagation();
    if (E.playing) R.monitor.pause();
    E.selectRegion(id);
    var regs = R.monitor.regionsFor(E.selSeg);
    var r = regs.filter(function (x) { return x.id === id; })[0];
    if (!r) return;
    var box = this.el.getBoundingClientRect();
    this.drag = { id: id, mode: mode, x0: e.clientX, y0: e.clientY, r0: r[this.which], bw: box.width, bh: box.height, moved: false,
      others: regs.filter(function (x) { return x.id !== id; }).map(function (x) { return x[this.which]; }, this) };
    var b = this.boxes[id];
    b.setPointerCapture(e.pointerId);
    var self = this;
    var move = function (ev) { self.move(ev); };
    var up = function (ev) {
      b.removeEventListener('pointermove', move); b.removeEventListener('pointerup', up); b.removeEventListener('pointercancel', up);
      self.up(ev);
    };
    b.addEventListener('pointermove', move); b.addEventListener('pointerup', up); b.addEventListener('pointercancel', up);
  };
  RectLayer.prototype.move = function (e) {
    var d = this.drag; if (!d) return;
    var dx = (e.clientX - d.x0) / d.bw, dy = (e.clientY - d.y0) / d.bh;
    if (!d.moved && Math.abs(e.clientX - d.x0) + Math.abs(e.clientY - d.y0) < 3) return;
    d.moved = true;
    var r = { x: d.r0.x, y: d.r0.y, w: d.r0.w, h: d.r0.h };
    var m = d.mode, snap = !e.altKey;
    var xs = [0, 0.5, 1], ys = [0, 0.5, 1];
    d.others.forEach(function (o) { xs.push(o.x, o.x + o.w); ys.push(o.y, o.y + o.h); });
    var gx = null, gy = null;
    if (m === 'move') {
      r.x = d.r0.x + dx; r.y = d.r0.y + dy;
      if (snap) {
        var cands = [[r.x, 0], [r.x + r.w, r.w], [r.x + r.w / 2, r.w / 2]], bx = null;
        cands.forEach(function (c) { var s = snapTo(c[0], xs); if (s !== c[0] && (bx === null || Math.abs(s - c[0]) < Math.abs(bx[0] - bx[1]))) bx = [s, c[0], c[1]]; });
        if (bx) { r.x = bx[0] - bx[2]; gx = bx[0]; }
        var candsY = [[r.y, 0], [r.y + r.h, r.h], [r.y + r.h / 2, r.h / 2]], by = null;
        candsY.forEach(function (c) { var s = snapTo(c[0], ys); if (s !== c[0] && (by === null || Math.abs(s - c[0]) < Math.abs(by[0] - by[1]))) by = [s, c[0], c[1]]; });
        if (by) { r.y = by[0] - by[2]; gy = by[0]; }
      }
      r.x = R.clamp(r.x, 0, 1 - r.w); r.y = R.clamp(r.y, 0, 1 - r.h);
    } else {
      var x1 = d.r0.x, y1 = d.r0.y, x2 = d.r0.x + d.r0.w, y2 = d.r0.y + d.r0.h;
      if (m.indexOf('w') >= 0) { x1 = R.clamp(x1 + dx, 0, x2 - MIN); if (snap) { var s1 = snapTo(x1, xs); if (s1 !== x1) { x1 = s1; gx = s1; } } }
      if (m.indexOf('e') >= 0) { x2 = R.clamp(x2 + dx, x1 + MIN, 1); if (snap) { var s2 = snapTo(x2, xs); if (s2 !== x2) { x2 = s2; gx = s2; } } }
      if (m.indexOf('n') >= 0) { y1 = R.clamp(y1 + dy, 0, y2 - MIN); if (snap) { var s3 = snapTo(y1, ys); if (s3 !== y1) { y1 = s3; gy = s3; } } }
      if (m.indexOf('s') >= 0) { y2 = R.clamp(y2 + dy, y1 + MIN, 1); if (snap) { var s4 = snapTo(y2, ys); if (s4 !== y2) { y2 = s4; gy = s4; } } }
      // Shift on a corner keeps the start aspect ratio (in pixels of this layer).
      if (e.shiftKey && m.length === 2) {
        var ar = (d.r0.w * d.bw) / (d.r0.h * d.bh);
        var wpx = (x2 - x1) * d.bw, hpx = wpx / ar;
        if (m.indexOf('n') >= 0) y1 = y2 - hpx / d.bh; else y2 = y1 + hpx / d.bh;
      }
      r = { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
    }
    r = clampRect(r);
    d.rect = r;
    var b = this.boxes[d.id];
    b.style.left = r.x * 100 + '%'; b.style.top = r.y * 100 + '%'; b.style.width = r.w * 100 + '%'; b.style.height = r.h * 100 + '%';
    this.showGuides(gx, gy);
    E.draft = { regionId: d.id, segId: E.selSeg, all: !!E.scopeAll, which: this.which, rect: r };
    E.emit('draft');
  };
  RectLayer.prototype.showGuides = function (gx, gy) {
    R.clear(this.guides);
    if (gx !== null && gx !== undefined) this.guides.appendChild(h('i', { class: 'gx', style: { left: gx * 100 + '%' } }));
    if (gy !== null && gy !== undefined) this.guides.appendChild(h('i', { class: 'gy', style: { top: gy * 100 + '%' } }));
  };
  RectLayer.prototype.up = function () {
    var d = this.drag; this.drag = null;
    this.showGuides(null, null);
    if (!d || !d.moved || !d.rect) return;
    var input = { region_id: d.id };
    input[this.which] = d.rect;
    if (!E.scopeAll) input.segment_id = E.selSeg;
    E.mutate('layout.edit_region', input);
  };

  // ------------------------------------------------------------ source panel
  G.mount = function (host, state) {
    E = state;
    info = h('span', { class: 'muted small ell' }, '');
    segSel = h('select', { class: 'small', title: 'segment whose layout you edit' });
    segSel.addEventListener('change', function () { E.selectSeg(segSel.value, true); });
    scopeChk = h('input', { type: 'checkbox' });
    scopeChk.addEventListener('change', function () { E.scopeAll = scopeChk.checked; E.emit('select'); });
    host.appendChild(h('div', { class: 'phead' }, h('strong', null, 'Source'), info, h('span', { class: 'grow' }), segSel,
      h('label', { class: 'small chk', title: 'region edits apply to every segment (same region ids)' }, scopeChk, 'all segments')));
    srcStage = h('div', { class: 'srcstage' });
    mediaBox = h('div', { class: 'mediabox' });
    var vid = R.monitor.video();
    mediaBox.appendChild(vid);
    srcLayer = new RectLayer(h('div', { class: 'rlayer' }), 'src');
    mediaBox.appendChild(srcLayer.el);
    suggestLayer = h('div', { class: 'suggest' });
    mediaBox.appendChild(suggestLayer);
    srcStage.appendChild(mediaBox);
    host.appendChild(srcStage);
    bar = h('div', { class: 'srcbar' });
    host.appendChild(bar);
    host.appendChild(h('div', { class: 'ptools' },
      R.button('+ Region', addRegion, { small: true, need: ['layout.add_region'], title: 'add a region (full source frame into the top half)' }),
      R.button('Detect webcam', detectWebcam, { small: true, need: ['layout.detect_webcam'], title: 'find a facecam box in the stream frame' }),
      R.button('Streamer split', startStreamerDraw, { small: true, need: ['layout.from_frame_boxes'], title: 'draw the webcam box, then the gameplay box' }),
      h('span', { class: 'grow' }),
      h('span', { class: 'muted small' }, 'drag boxes · Alt = no snap')));
    G.fitSrc = R.fitBox(srcStage, mediaBox, function () {
      var m = E.clip && E.segAt(E.t); var s = m && E.sources[m.source_id]; var p = (s && s.probe) || {};
      return p.width && p.height ? p.width / p.height : 16 / 9;
    });
    dstLayerObj = new RectLayer(R.monitor.dstLayer, 'dst');
    E.on('load', function () { E.draft = null; refresh(); });
    E.on('select', refresh);
    E.on('draft', function () { if (E.draft && E.draft.which === 'dst') srcLayer.update(R.monitor.regionsFor(E.selSeg), E.selRegion); });
    E.on('time', function () {
      var m = E.segAt(E.t);
      if (m && m.segment_id !== E.selSeg && !E.playing) { /* selection follows clicks, not playback */ }
    });
  };
  function refresh() {
    if (!E || !E.clip) return;
    R.clear(segSel);
    E.timeline.forEach(function (m, i) { segSel.appendChild(R.option(m.segment_id, 'seg ' + (i + 1) + ' · ' + R.fmt(m.out_start) + '-' + R.fmt(m.out_end), m.segment_id === E.selSeg)); });
    segSel.hidden = E.timeline.length < 2;
    var uni = E.uniformLayout();
    scopeChk.checked = !!E.scopeAll; scopeChk.disabled = !uni || E.timeline.length < 2;
    scopeChk.parentNode.hidden = E.timeline.length < 2;
    var seg = E.segById(E.selSeg);
    var src = seg && E.sources[seg.source_id];
    var lay = seg && seg.layout;
    info.textContent = (src ? (src.name || seg.source_id) : '') + (lay ? '  ·  ' + (lay.preset || 'custom') + '  ·  ' + lay.regions.length + ' region' + (lay.regions.length === 1 ? '' : 's') : '');
    var regs = R.monitor.regionsFor(E.selSeg);
    srcLayer.update(regs, E.selRegion);
    if (G.fitSrc) G.fitSrc();
    R.monitor.draw();
  }
  G.drawDst = function (segId, regs) {
    if (!dstLayerObj || !E) return;
    if (segId !== E.selSeg && !E.scopeAll) { dstLayerObj.update([], ''); return; }
    dstLayerObj.update(regs, E.selRegion);
  };
  G.refresh = function () { refresh(); };

  // ------------------------------------------------------------ actions
  function addRegion() {
    var lay = E.layoutOf(E.selSeg);
    var n = lay ? lay.regions.length : 0;
    var input = { src: { x: 0, y: 0, w: 1, h: 1 }, dst: n ? { x: 0.05, y: 0.05, w: 0.4, h: 0.225 } : { x: 0, y: 0, w: 1, h: 1 }, fit: 'cover' };
    if (!E.scopeAll) input.segment_id = E.selSeg;
    return E.mutate('layout.add_region', input).then(function (d) { if (d && d.region_id) { E.selectRegion(d.region_id); R.toast('Added region ' + d.region_id + ': drag its boxes into place'); } });
  }
  function clearSuggest() { R.clear(suggestLayer); suggestLayer.classList.remove('drawing'); R.clear(bar); }
  function sBox(rect, label, cls) {
    return h('div', { class: 'sbox ' + (cls || ''), style: { left: rect.x * 100 + '%', top: rect.y * 100 + '%', width: rect.w * 100 + '%', height: rect.h * 100 + '%' } }, h('span', null, label));
  }
  function detectWebcam() {
    var seg = E.segById(E.selSeg);
    if (!seg) return;
    var m = E.tlById(seg.id);
    var input = { source_id: seg.source_id, clip_id: E.clipId };
    if (!E.scopeAll) input.segment_id = seg.id;
    clearSuggest();
    bar.appendChild(h('span', { class: 'muted small' }, 'Looking for a facecam (face tracking over a few frames)...'));
    return R.run('layout.detect_webcam', input).then(function (d) {
      clearSuggest();
      if (!d) return;
      suggestLayer.appendChild(sBox(d.webcam, 'webcam (' + d.position + ')', 'cam'));
      suggestLayer.appendChild(sBox(d.gameplay, 'gameplay', 'game'));
      if (d.face) suggestLayer.appendChild(sBox(d.face, 'face', 'face'));
      var apply = Object.assign({}, d.from_frame_boxes_input || {});
      delete apply.base_revision;
      if (E.scopeAll) delete apply.segment_id; else apply.segment_id = seg.id;
      bar.appendChild(h('div', { class: 'row' },
        h('span', { class: 'small' }, 'Webcam ' + d.position + ' · confidence ' + R.round(d.confidence, 2) + ' · ' + d.face_hits + '/' + (d.times || []).length + ' frames'),
        h('span', { class: 'grow' }),
        R.button('Apply split', function () { return E.mutate('layout.from_frame_boxes', apply, { toast: 'Streamer split applied' }).then(function (r) { if (r) clearSuggest(); }); }, { small: true, primary: true }),
        R.button('Dismiss', clearSuggest, { small: true })));
      (d.warnings || []).forEach(function (w) { R.toast(w); });
      void m;
    });
  }
  // Draw two boxes on the source frame: webcam, then gameplay -> layout.from_frame_boxes.
  function startStreamerDraw() {
    if (E.playing) R.monitor.pause();
    clearSuggest();
    var boxes = [];
    var labels = ['webcam (top pane)', 'gameplay (bottom pane)'];
    var cur = null, start = null;
    var msg = h('span', { class: 'small' });
    var row = h('div', { class: 'row' }, msg, h('span', { class: 'grow' }), R.button('Cancel', function () { suggestLayer.removeEventListener('pointerdown', down); clearSuggest(); }, { small: true }));
    bar.appendChild(row);
    function prompt() { msg.textContent = boxes.length < 2 ? 'Step ' + (boxes.length + 1) + '/2: drag a box around the ' + labels[boxes.length] + ' on the source frame.' : 'Both boxes drawn.'; }
    prompt();
    suggestLayer.classList.add('drawing');
    function pos(e) { var b = suggestLayer.getBoundingClientRect(); return { x: R.clamp((e.clientX - b.left) / b.width, 0, 1), y: R.clamp((e.clientY - b.top) / b.height, 0, 1) }; }
    function down(e) {
      if (boxes.length >= 2 || e.button !== 0) return;
      e.preventDefault();
      start = pos(e);
      cur = sBox({ x: start.x, y: start.y, w: 0, h: 0 }, labels[boxes.length], boxes.length ? 'game' : 'cam');
      suggestLayer.appendChild(cur);
      suggestLayer.setPointerCapture(e.pointerId);
      suggestLayer.addEventListener('pointermove', mv);
      suggestLayer.addEventListener('pointerup', upf);
    }
    function rectFrom(e) { var p = pos(e); return clampRect({ x: Math.min(p.x, start.x), y: Math.min(p.y, start.y), w: Math.abs(p.x - start.x), h: Math.abs(p.y - start.y) }); }
    function mv(e) { var r = rectFrom(e); cur.style.left = r.x * 100 + '%'; cur.style.top = r.y * 100 + '%'; cur.style.width = r.w * 100 + '%'; cur.style.height = r.h * 100 + '%'; }
    function upf(e) {
      suggestLayer.removeEventListener('pointermove', mv); suggestLayer.removeEventListener('pointerup', upf);
      var r = rectFrom(e);
      if (r.w < 0.03 || r.h < 0.03) { suggestLayer.removeChild(cur); return; }
      boxes.push(r); prompt();
      if (boxes.length === 2) {
        suggestLayer.removeEventListener('pointerdown', down);
        suggestLayer.classList.remove('drawing');
        var input = { boxes: [{ role: 'top', rect: boxes[0] }, { role: 'bottom', rect: boxes[1] }] };
        if (!E.scopeAll) input.segment_id = E.selSeg;
        row.insertBefore(R.button('Apply split', function () {
          return E.mutate('layout.from_frame_boxes', input, { toast: 'Streamer split applied' }).then(function (d) { if (d) clearSuggest(); });
        }, { small: true, primary: true }), row.lastChild);
      }
    }
    suggestLayer.addEventListener('pointerdown', down);
  }
  G.addRegion = addRegion; G.detectWebcam = detectWebcam; G.streamerSplit = startStreamerDraw;
})();
