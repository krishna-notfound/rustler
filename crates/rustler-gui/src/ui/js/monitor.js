// Program monitor: plays the source proxy through the clip timeline (seek by output time) and
// composites a live 9:16 preview from the layout regions with <canvas> drawImage. The exact look
// (libass captions, encoder output) is one click away: "Exact frame" = render.preview_frame.
(function () {
  'use strict';
  var R = window.R, h = R.h;
  var M = R.monitor = {};
  var E = null, video = null, canvas = null, ctx = null, raf = 0, exactImg = null, tcEl = null, playBtn = null, srcKey = '';
  var dstLayer = null, showRegions = R.store('rustler.showDst') !== '0';
  var lastEmit = 0;

  function mediaPath(sid) {
    var s = E.sources[sid] || {};
    return s.proxy || s.media_path || '';
  }
  // Load the right source into the shared <video>; resolves when it can seek.
  function ensureSource(sid) {
    var p = mediaPath(sid);
    if (!p) return Promise.resolve(false);
    if (srcKey === sid && video.readyState >= 1) return Promise.resolve(true);
    return new Promise(function (res) {
      var done = function () { video.removeEventListener('loadedmetadata', done); video.removeEventListener('error', fail); res(true); };
      var fail = function () { video.removeEventListener('loadedmetadata', done); video.removeEventListener('error', fail); res(false); };
      video.addEventListener('loadedmetadata', done);
      video.addEventListener('error', fail);
      srcKey = sid;
      video.src = R.fileUrl(p);
    });
  }

  M.mount = function (host, state) {
    E = state;
    video = h('video', { preload: 'auto', playsinline: true, class: 'srcvideo', crossorigin: 'anonymous' });
    video.muted = R.store('rustler.muted') === '1';
    canvas = h('canvas', { class: 'progcanvas', width: 405, height: 720 });
    ctx = canvas.getContext('2d');
    exactImg = h('div', { class: 'exact', hidden: true });
    dstLayer = h('div', { class: 'rlayer' });
    tcEl = h('span', { class: 'tc' }, '00:00.00');
    playBtn = h('button', { class: 'small primary', type: 'button', title: 'Space', onclick: function () { M.toggle(); } }, '▶');
    var mute = h('button', { class: 'small ghost', type: 'button', title: 'mute / unmute', onclick: function () { video.muted = !video.muted; R.store('rustler.muted', video.muted ? '1' : '0'); mute.textContent = video.muted ? '🔇' : '🔊'; } }, video.muted ? '🔇' : '🔊');
    var regionsChk = h('input', { type: 'checkbox', checked: showRegions, onchange: function () { showRegions = regionsChk.checked; R.store('rustler.showDst', showRegions ? '1' : '0'); dstLayer.hidden = !showRegions; } });
    var stage = h('div', { class: 'progstage' }, h('div', { class: 'canvasbox' }, canvas, dstLayer, exactImg));
    host.appendChild(h('div', { class: 'phead' }, h('strong', null, 'Program'), h('span', { class: 'muted small' }, '9:16 live preview'),
      h('span', { class: 'grow' }), h('label', { class: 'small chk', title: 'show and drag destination rects on the canvas' }, regionsChk, 'regions')));
    host.appendChild(stage);
    host.appendChild(h('div', { class: 'transport' },
      h('button', { class: 'small ghost', type: 'button', title: 'start (Home)', onclick: function () { M.seek(0); } }, '⏮'),
      h('button', { class: 'small ghost', type: 'button', title: '1 frame back (←)', onclick: function () { M.seek(E.t - 1 / E.fps); } }, '◀'),
      playBtn,
      h('button', { class: 'small ghost', type: 'button', title: '1 frame forward (→)', onclick: function () { M.seek(E.t + 1 / E.fps); } }, '▶▶'),
      tcEl, mute,
      h('span', { class: 'grow' }),
      R.button('Exact frame', function () { return M.exact(); }, { small: true, need: ['render.preview_frame'], title: 'render.preview_frame: the real renderer (libass captions) at the playhead' })));
    dstLayer.hidden = !showRegions;
    M.dstLayer = dstLayer;
    M.canvasBox = stage.firstChild;
    M.refit = R.fitBox(stage, stage.firstChild, function () { return E && E.clip ? E.clip.canvas.width / E.clip.canvas.height : 9 / 16; });
    // the <video> itself is shown by the Source panel (regions.js)
    video.addEventListener('seeked', draw);
    video.addEventListener('loadeddata', draw);
    video.addEventListener('canplay', function () { if (!E || !E.playing) requestAnimationFrame(draw); });
    video.addEventListener('ended', function () { M.pause(); });
    E.on('load', function (reason) {
      var m = E.segAt(E.t);
      if (m) ensureSource(m.source_id).then(function () { if (!E.playing) M.seek(E.t, true); });
      if (reason === 'first') sizeCanvas();
      draw();
    });
    E.on('captions', draw);
    E.on('select', draw);
    E.on('draft', draw);
    window.addEventListener('resize', sizeCanvas);
  };
  M.unmount = function () {
    cancelAnimationFrame(raf);
    window.removeEventListener('resize', sizeCanvas);
    if (video) { try { video.pause(); video.removeAttribute('src'); video.load(); } catch (e) { /* ignore */ } }
    E = null; video = null; srcKey = '';
  };
  M.video = function () { return video; };

  function sizeCanvas() {
    if (!E || !E.clip) return;
    var cw = E.clip.canvas.width, ch = E.clip.canvas.height;
    var scale = Math.min(1, 720 / ch);
    canvas.width = Math.round(cw * scale); canvas.height = Math.round(ch * scale);
    if (M.refit) M.refit();
    draw();
  }

  // ------------------------------------------------------------ transport
  M.seek = function (t, quiet) {
    if (!E) return;
    t = R.clamp(t, 0, Math.max(0, E.duration - 0.001));
    E.t = t;
    hideExact();
    var m = E.outToSrc(t);
    if (m) {
      if (!quiet && !E.playing && E.selSeg !== m.seg.segment_id) E.selectSeg(m.seg.segment_id);
      ensureSource(m.seg.source_id).then(function () {
        if (!video || Math.abs(video.currentTime - m.srcT) < 0.004) { draw(); return; }
        video.currentTime = m.srcT;
      });
    }
    tcEl.textContent = R.tc(t, E.fps);
    E.emit('time', t);
  };
  M.play = function () {
    if (!E || !E.timeline.length) return;
    if (E.t >= E.duration - 0.05) E.t = 0;
    var m = E.outToSrc(E.t);
    hideExact();
    ensureSource(m.seg.source_id).then(function () {
      if (Math.abs(video.currentTime - m.srcT) > 0.05) video.currentTime = m.srcT;
      E.playing = true;
      playBtn.textContent = '❚❚';
      var p = video.play();
      if (p && p.catch) p.catch(function () { M.pause(); });
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(loop);
    });
  };
  M.pause = function () {
    if (!E) return;
    E.playing = false;
    if (video) video.pause();
    if (playBtn) playBtn.textContent = '▶';
    cancelAnimationFrame(raf);
    E.emit('time', E.t);
  };
  M.toggle = function () { if (E && E.playing) M.pause(); else M.play(); };

  var switching = false;
  function loop() {
    if (!E || !E.playing) return;
    var m = E.segAt(E.t);
    if (m && !switching && srcKey === m.source_id) {
      var vt = video.currentTime;
      var t = m.out_start + (vt - m.src_start);
      if (vt >= m.src_end - 0.5 / E.fps || t >= m.out_end) {
        // next segment (may be another source or a jump within the same source)
        var ix = E.timeline.indexOf(m);
        var nx = E.timeline[ix + 1];
        if (!nx) { E.t = Math.max(0, E.duration - 0.001); M.pause(); draw(); return; }
        E.t = nx.out_start;
        switching = true;
        var resume = function () { switching = false; };
        if (nx.source_id !== srcKey) {
          video.pause();
          ensureSource(nx.source_id).then(function () { video.currentTime = nx.src_start; return video.play(); }).then(resume, resume);
        } else { video.currentTime = nx.src_start; resume(); }
      } else if (t >= m.out_start - 0.2) {
        E.t = Math.max(m.out_start, t);
      }
    }
    tcEl.textContent = R.tc(E.t, E.fps);
    var now = performance.now();
    if (now - lastEmit > 50) { lastEmit = now; E.emit('time', E.t); }
    draw();
    raf = requestAnimationFrame(loop);
  }

  // ------------------------------------------------------------ compositing
  function cropX(keys, t) {
    if (!keys || !keys.length) return null;
    if (t <= keys[0].t) return keys[0].x;
    for (var i = 1; i < keys.length; i++) {
      if (t <= keys[i].t) { var a = keys[i - 1], b = keys[i]; var f = (t - a.t) / Math.max(1e-6, b.t - a.t); return a.x + (b.x - a.x) * f; }
    }
    return keys[keys.length - 1].x;
  }
  // Region list with the in-progress drag (E.draft) applied.
  M.regionsFor = function (segId) {
    var lay = E.layoutOf(segId);
    if (!lay) return [];
    return lay.regions.map(function (r, ix) {
      var d = E.draft;
      if (d && d.regionId === r.id && (d.all || d.segId === segId)) {
        var c = { id: r.id, src: r.src, dst: r.dst, z: r.z, fit: r.fit, ix: ix };
        c[d.which] = d.rect;
        return c;
      }
      return { id: r.id, src: r.src, dst: r.dst, z: r.z, fit: r.fit, ix: ix };
    });
  };
  function draw() {
    if (!E || !E.clip || !ctx) return;
    var W = canvas.width, H = canvas.height;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, W, H);
    var m = E.segAt(E.t);
    if (!m) return;
    var lay = E.layoutOf(m.segment_id);
    var ready = video && video.readyState >= 2 && srcKey === m.source_id;
    var vw = ready ? video.videoWidth : 0, vh = ready ? video.videoHeight : 0;
    var regs = M.regionsFor(m.segment_id);
    var order = regs.slice().sort(function (a, b) { return a.z - b.z; });
    var full = function (r) { return r.fit === 'cover' && r.dst.x <= 0.001 && r.dst.y <= 0.001 && r.dst.w >= 0.999 && r.dst.h >= 0.999; };
    var keys = E.extras && E.extras.crop_paths ? E.extras.crop_paths[m.segment_id] : null;
    var segT = E.t - m.out_start;
    if (ready) {
      if (lay && lay.background === 'blur_fill' && !(order[0] && full(order[0]))) {
        var sc = Math.max(W / vw, H / vh);
        var bw = W / sc, bh = H / sc;
        ctx.save();
        ctx.filter = 'blur(' + Math.round(W / 40) + 'px) brightness(0.6)';
        ctx.drawImage(video, (vw - bw) / 2, (vh - bh) / 2, bw, bh, -10, -10, W + 20, H + 20);
        ctx.restore();
      }
      order.forEach(function (r) {
        var s = { x: r.src.x * vw, y: r.src.y * vh, w: r.src.w * vw, h: r.src.h * vh };
        if (r.ix === 0 && keys && keys.length) { var cx = cropX(keys, segT); if (cx !== null) s.x = R.clamp(cx, 0, 1 - r.src.w) * vw; }
        var d = { x: r.dst.x * W, y: r.dst.y * H, w: r.dst.w * W, h: r.dst.h * H };
        if (s.w < 1 || s.h < 1 || d.w < 1 || d.h < 1) return;
        if (r.fit === 'contain') {
          var k = Math.min(d.w / s.w, d.h / s.h);
          var dw = s.w * k, dh = s.h * k;
          ctx.drawImage(video, s.x, s.y, s.w, s.h, d.x + (d.w - dw) / 2, d.y + (d.h - dh) / 2, dw, dh);
        } else {
          var k2 = Math.max(d.w / s.w, d.h / s.h);
          var cw = d.w / k2, ch = d.h / k2;
          ctx.drawImage(video, s.x + (s.w - cw) / 2, s.y + (s.h - ch) / 2, cw, ch, d.x, d.y, d.w, d.h);
        }
      });
    }
    drawEffectTint(W, H);
    drawOverlays(W, H);
    drawCaption(W, H);
    drawProgressBar(W, H);
    if (showRegions) R.regions && R.regions.drawDst && R.regions.drawDst(m.segment_id, regs);
  }
  M.draw = draw;

  // Progress bar (overlay.progress_bar): fills the canvas width over the clip, on top of everything.
  function drawProgressBar(W, H) {
    var pb = E.progressBar;
    if (!pb || !E.duration) return;
    var k = W / E.clip.canvas.width;
    var bh = Math.max(2, (pb.height || 8) * k);
    var frac = R.clamp(E.t / E.duration, 0, 1);
    ctx.fillStyle = pb.color || '#FFE000';
    ctx.fillRect(0, pb.position === 'bottom' ? H - bh : 0, W * frac, bh);
  }
  // Effects (effect.add) are applied by the renderer; the live preview marks the active one.
  function drawEffectTint(W, H) {
    var fx = (E.effects || []).filter(function (f) { return E.t >= f.start && E.t < f.end; })[0];
    if (!fx) return;
    ctx.save();
    if (fx.kind === 'flash') { ctx.fillStyle = 'rgba(255,255,255,.55)'; ctx.fillRect(0, 0, W, H); }
    ctx.strokeStyle = 'rgba(255,138,61,.9)'; ctx.lineWidth = 3; ctx.setLineDash([8, 6]);
    ctx.strokeRect(6, 6, W - 12, H - 12);
    ctx.setLineDash([]);
    ctx.font = '600 12px "Segoe UI", Arial, sans-serif'; ctx.fillStyle = 'rgba(0,0,0,.6)';
    var label = 'FX ' + fx.kind + (fx.kind !== 'flash' ? ' ' + R.round(fx.amount_used || fx.amount || 0, 3) : '');
    var tw = ctx.measureText(label).width;
    ctx.fillRect(10, H - 30, tw + 12, 20);
    ctx.fillStyle = '#ffb27a'; ctx.fillText(label, 16, H - 15);
    ctx.restore();
  }

  function drawCaption(W, H) {
    var cap = E.clip.captions;
    if (!cap || !cap.enabled || !E.captions.length) return;
    var cue = null;
    for (var i = 0; i < E.captions.length; i++) { var c = E.captions[i]; if (E.t >= c.start && E.t < c.end) { cue = c; break; } }
    if (!cue) return;
    // Mirrors rustler-captions presets (font, size = canvas width x factor, outline, active word).
    var P = PRESETS[cap.preset] || customPreset(cap.preset) || PRESETS['clean-white'];
    // caption.speaker_colors: the cue takes the colour of its speaker (words labelled by analyze.speakers)
    var sc = E.extras && E.extras.speaker_colors;
    if (sc && sc.enabled && R.timeline && R.timeline.outWords) {
      var w0 = R.timeline.outWords().filter(function (w) { return w.t0 >= cue.start - 0.02 && w.t0 < cue.end; })[0];
      if (w0 && w0.speaker && sc.colors && sc.colors[w0.speaker]) P = Object.assign({}, P, { primary: sc.colors[w0.speaker] });
    }
    var k = W / E.clip.canvas.width;
    // ASS font size ~ line height; CSS px = em (ratios measured against libass renders)
    var size = E.clip.canvas.width * P.size * k / (P.font === 'anton' ? 1.7 : 1.4);
    ctx.save();
    ctx.font = (P.font === 'anton' ? '400 ' : '800 ') + Math.round(size) + 'px ' + R.fonts[P.font];
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    var words = cue.text.split(/\s+/).filter(Boolean).map(function (w) { return P.upper ? w.toUpperCase() : w; });
    var active = activeWordIndex(cue, words.length);
    var space = ctx.measureText(' ').width;
    // lines of words wrapped to 90% width
    var lines = [], cur = [], curW = 0;
    words.forEach(function (w, i) {
      var ww = ctx.measureText(w).width;
      if (cur.length && curW + space + ww > W * 0.9) { lines.push({ ws: cur, w: curW }); cur = []; curW = 0; }
      curW += (cur.length ? space : 0) + ww; cur.push({ t: w, w: ww, i: i });
    });
    if (cur.length) lines.push({ ws: cur, w: curW });
    var lh = size * 1.2;
    var y = (cap.position_y === undefined ? 0.7 : cap.position_y) * H - (lines.length - 1) * lh;
    lines.forEach(function (ln, j) {
      var x = W / 2 - ln.w / 2, yy = y + j * lh;
      ln.ws.forEach(function (w) {
        var on = w.i === active;
        if (on && P.active === 'box') {
          var pad = 12 * k;
          ctx.fillStyle = P.color;
          ctx.fillRect(x - pad, yy - size * 0.95, w.w + 2 * pad, size * 1.25);
        }
        if (P.outline) { ctx.lineWidth = P.outline * k * 2; ctx.strokeStyle = P.stroke || '#000'; ctx.lineJoin = 'round'; ctx.strokeText(w.t, x, yy); }
        ctx.fillStyle = on && P.active !== 'box' ? P.color : (P.primary || '#fff');
        if (on && P.active === 'pop') { ctx.save(); ctx.translate(x + w.w / 2, yy); ctx.scale(1.08, 1.08); ctx.fillText(w.t, -w.w / 2, 0); ctx.restore(); }
        else ctx.fillText(w.t, x, yy);
        x += w.w + space;
      });
    });
    ctx.restore();
  }
  var PRESETS = {
    'bold-karaoke': { font: 'anton', size: 0.072, upper: true, outline: 6, active: 'recolor', color: '#FFE000' },
    'clean-white': { font: 'poppins', size: 0.060, upper: false, outline: 4, active: 'recolor', color: '#FFD700' },
    'boxed-highlight': { font: 'poppins', size: 0.058, upper: false, outline: 3, active: 'box', color: '#7B2CFF' },
    'hormozi-pop': { font: 'anton', size: 0.076, upper: true, outline: 6, active: 'pop', color: '#2BFF5E' }
  };
  // Custom styles (caption.create_style): approximate from caption.styles (R.styleDefs); exact look = Exact frame.
  function customPreset(name) {
    var d = R.styleDefs && R.styleDefs[name];
    if (!d) return null;
    var base = PRESETS[d.base] || (/anton/i.test(d.font_family || '') ? PRESETS['bold-karaoke'] : PRESETS['clean-white']);
    return { font: /anton/i.test(d.font_family || '') ? 'anton' : 'poppins', size: d.size_factor || base.size, upper: d.uppercase !== undefined ? !!d.uppercase : base.upper,
      outline: d.outline !== undefined ? d.outline : base.outline, active: d.active_kind || base.active, color: d.active_colour || base.color,
      primary: d.primary || '#fff', stroke: d.outline_colour || '#000' };
  }
  // Index (within the cue) of the word being spoken, from the transcript words in output time.
  function activeWordIndex(cue, n) {
    if (!R.timeline || !R.timeline.outWords) return -1;
    var ws = R.timeline.outWords().filter(function (w) { return w.t0 >= cue.start - 0.02 && w.t0 < cue.end - 0.01; });
    for (var i = 0; i < ws.length && i < n; i++) if (E.t >= ws[i].t0 && (i === ws.length - 1 || E.t < ws[i + 1].t0)) return i;
    return -1;
  }
  function wrap(text, maxW) {
    var words = text.split(/\s+/), lines = [], cur = '';
    words.forEach(function (w) {
      var t = cur ? cur + ' ' + w : w;
      if (ctx.measureText(t).width > maxW && cur) { lines.push(cur); cur = w; } else cur = t;
    });
    if (cur) lines.push(cur);
    return lines;
  }
  // Text overlays (overlay.* commands): approximate boxes so timing is visible while editing.
  function drawOverlays(W, H) {
    if (!E.overlays || !E.overlays.length) return;
    E.overlays.forEach(function (o) {
      var tm = R.overlayTimes ? R.overlayTimes(o) : null;
      if (!tm || E.t < tm.start || E.t >= tm.end) return;
      var text = o.text || o.title || '';
      if (!text) return;
      var L = o.layout;
      if (L && Array.isArray(L.lines) && L.font_size) {
        // overlay.list gives the renderer's own text layout in canvas pixels: mirror it.
        var k = W / E.clip.canvas.width;
        var fs = L.font_size * k, pad = (L.padding || 0) * k, lh = (L.bottom - L.top - 2 * (L.padding || 0)) * k / Math.max(1, L.lines.length);
        ctx.save();
        fs = fs / 1.4;   // ASS size ~ line height; CSS px = em
        ctx.font = '800 ' + Math.round(fs) + 'px ' + R.fonts.poppins;
        // never wider than the renderer's measured box
        var widest = Math.max.apply(null, L.lines.map(function (ln) { return ctx.measureText(ln).width; }));
        var avail = (L.width - 2 * (L.padding || 0)) * k;
        if (widest > avail) { fs = fs * avail / widest; ctx.font = '800 ' + Math.round(fs) + 'px ' + R.fonts.poppins; }
        ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
        if (o.box) { ctx.fillStyle = 'rgba(0,0,0,.82)'; ctx.fillRect(W / 2 - (L.width * k) / 2, L.top * k, L.width * k, (L.bottom - L.top) * k); }
        L.lines.forEach(function (ln, j) {
          var yy = L.top * k + pad + lh * (j + 0.5);
          if (!o.box) { ctx.lineWidth = Math.max(2, fs / 8); ctx.strokeStyle = '#000'; ctx.strokeText(ln, W / 2, yy); }
          ctx.fillStyle = '#fff';
          ctx.fillText(ln, W / 2, yy);
        });
        ctx.restore();
        return;
      }
      var size = Math.round(H * (o.font_size ? o.font_size / 1920 : 0.045));
      ctx.save();
      ctx.font = '800 ' + size + 'px "Segoe UI", Arial, sans-serif';
      ctx.textAlign = 'center';
      var y = (typeof o.position_y === 'number' ? o.position_y : typeof o.y === 'number' ? o.y : (/lower|bottom/.test(String(o.kind || o.style || o.position || '')) ? 0.8 : 0.13)) * H;
      var lines = wrap(text, W * 0.84);
      lines.forEach(function (ln, j) {
        var w = ctx.measureText(ln).width, yy = y + j * size * 1.2;
        ctx.fillStyle = 'rgba(255,255,255,.95)';
        ctx.fillRect(W / 2 - w / 2 - size * 0.4, yy - size, w + size * 0.8, size * 1.3);
        ctx.fillStyle = '#111';
        ctx.fillText(ln, W / 2, yy);
      });
      ctx.restore();
    });
  }

  // ------------------------------------------------------------ exact frame
  function hideExact() { if (exactImg && !exactImg.hidden) { exactImg.hidden = true; R.clear(exactImg); } }
  M.exact = function () {
    if (!E) return;
    M.pause();
    var t = R.round(E.t, 3);
    return R.run('render.preview_frame', { clip_id: E.clipId, out_t: t }).then(function (d) {
      if (!d || !d.image) return;
      R.clear(exactImg);
      exactImg.appendChild(h('img', { src: R.fileUrl(d.image) + '&v=' + Date.now(), alt: 'exact frame' }));
      exactImg.appendChild(h('div', { class: 'exact-bar' }, h('span', null, 'Exact frame @ ' + R.tc(t, E.fps) + ' (libass)'),
        h('button', { class: 'small', type: 'button', onclick: hideExact }, 'Back to live')));
      exactImg.hidden = false;
      return d;
    });
  };
})();
