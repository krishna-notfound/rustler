// Editor inspector panels built on the real commands: Audio (audio.*), Effects (transition.*,
// effect.*, overlay.progress_bar), QC (qc.run, fix.suggest|apply, qc.compare), Variants
// (variant.*, render.batch, export.batch). inspector.js mounts them as tabs; every change goes
// through E.mutate (revision guard + reload) so the timeline and monitor follow.
(function () {
  'use strict';
  var R = window.R, h = R.h;
  var P = R.panels = {};
  var TRANSITIONS = ['cut', 'fade', 'dissolve', 'wipeleft', 'wiperight', 'slideup', 'zoomin'];
  var EFFECTS = ['punch-in', 'zoom-slow', 'shake', 'flash'];

  function sec(title, extra) { return h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, title), extra || null)); }
  function base(p) { return String(p || '').split(/[\\/]/).pop(); }
  function num(v, step, min, max, cls) { return h('input', { type: 'number', step: step || 'any', min: min, max: max, value: v === undefined || v === null ? '' : v, class: cls || 'num' }); }

  // Library assets of one kind (cached per editor session).
  var libCache = {};
  P.library = function (kind) {
    if (!R.has('asset.list')) return Promise.resolve([]);
    if (!libCache[kind]) libCache[kind] = R.run('asset.list', { kind: kind }, true).then(function (d) { return R.listOf(d, 'assets'); });
    return libCache[kind];
  };
  P.resetLibrary = function () { libCache = {}; };

  // ------------------------------------------------------------ Audio
  P.audio = function (el, E) {
    var a = E.audioList || {};
    // summary
    var sum = sec('Mix');
    el.appendChild(sum);
    sum.appendChild(h('div', { class: 'kv' },
      h('span', null, 'Edit length'), h('b', { class: 'mono' }, R.tc(a.duration !== undefined ? a.duration : E.duration, E.fps)),
      h('span', null, 'Rendered length'), h('b', { class: 'mono', 'data-k': 'rendered' }, R.tc(a.rendered_duration !== undefined ? a.rendered_duration : E.duration, E.fps)),
      h('span', null, 'Loudness'), h('b', null, a.loudnorm === false ? 'off' : 'two-pass loudnorm -14 LUFS'),
      h('span', null, 'Denoise'), h('b', { 'data-k': 'denoise' }, a.denoise || 'off'),
      h('span', null, 'Dialogue gain'), h('b', null, (a.volume_db || 0) + ' dB'),
      h('span', null, 'Music'), h('b', null, a.music ? base(a.music.path) : 'none'),
      h('span', null, 'Sound effects'), h('b', null, String((a.sfx || []).length))));
    (a.warnings || []).forEach(function (w) { sum.appendChild(h('div', { class: 'small st-warn' }, w)); });

    // denoise
    if (R.has('audio.denoise')) {
      var dn = sec('Denoise (dialogue clean-up)');
      el.appendChild(dn);
      var seg = h('div', { class: 'segbtns' });
      ['off', 'light', 'medium', 'strong'].forEach(function (s) {
        seg.appendChild(h('button', { type: 'button', class: 'small' + ((a.denoise || 'off') === s ? ' on' : ''), 'data-strength': s, onclick: function () {
          E.mutate('audio.denoise', { strength: s }, { toast: 'Denoise: ' + s });
        } }, s));
      });
      dn.appendChild(seg);
      dn.appendChild(h('div', { class: 'muted small' }, '80 Hz high-pass + FFT denoise tuned to the measured noise floor, before loudnorm.'));
    }

    // volume
    if (R.has('audio.volume')) {
      var vs = sec('Volume');
      el.appendChild(vs);
      var all = num(a.volume_db || 0, 0.5, -60, 24);
      vs.appendChild(h('div', { class: 'row' }, h('span', { class: 'small grow' }, 'Whole dialogue'), all, h('span', { class: 'muted small' }, 'dB'),
        R.button('Set', function () { return E.mutate('audio.volume', { db: Number(all.value) || 0 }, { toast: 'Dialogue gain ' + (Number(all.value) || 0) + ' dB' }); }, { small: true })));
      var cur = {};
      (a.segment_volumes || []).forEach(function (s) { cur[s.segment_id] = s.db; });
      E.timeline.forEach(function (m, i) {
        var inp = num(cur[m.segment_id] || 0, 0.5, -60, 24);
        vs.appendChild(h('div', { class: 'row segvol' + (m.segment_id === E.selSeg ? ' sel' : ''), 'data-seg': m.segment_id },
          h('span', { class: 'small grow' }, h('b', null, 'Seg ' + (i + 1)), ' ', h('span', { class: 'muted mono' }, R.fmt(m.out_start) + '–' + R.fmt(m.out_end))),
          inp, h('span', { class: 'muted small' }, 'dB'),
          R.button('Set', function () { return E.mutate('audio.volume', { segment_id: m.segment_id, db: Number(inp.value) || 0 }, { toast: 'Segment ' + (i + 1) + ': ' + (Number(inp.value) || 0) + ' dB' }); }, { small: true })));
      });
    }

    // music
    var ms = sec('Music bed');
    el.appendChild(ms);
    var mu = a.music || (E.extras && E.extras.music);
    if (mu) {
      ms.appendChild(h('div', { class: 'row' }, h('span', { class: 'music-pill' }, '♫ ' + base(mu.path)), h('span', { class: 'muted small' }, mu.volume_db + ' dB' + (mu.duck ? ' · ducked' : '') + ' · fades ' + mu.fade_in + '/' + mu.fade_out + ' s'),
        h('span', { class: 'grow' }), R.button('Remove', function () { return E.mutate('audio.remove_music', {}, { toast: 'Music removed' }); }, { small: true, cls: 'ghost danger', need: ['audio.remove_music'] })));
    }
    if (R.has('audio.add_music')) {
      var pick = h('select', null, R.option('', 'loading library...'));
      var path = h('input', { type: 'text', placeholder: 'or a local file path', class: 'grow' });
      var vol = num(mu ? mu.volume_db : -18, 0.5, -60, 12);
      var duck = h('input', { type: 'checkbox', checked: mu ? !!mu.duck : true });
      var fi = num(mu ? mu.fade_in : 1, 0.1, 0, 10), fo = num(mu ? mu.fade_out : 1.5, 0.1, 0, 10);
      var player = h('audio', { controls: true, preload: 'none', class: 'miniplayer', hidden: true });
      P.library('music').then(function (list) {
        R.clear(pick);
        pick.appendChild(R.option('', list.length ? '(pick from library)' : 'no music in the library'));
        list.forEach(function (x) { pick.appendChild(R.option(x.id, x.name + ' · ' + (x.license || ''))); });
      });
      pick.addEventListener('change', function () {
        P.library('music').then(function (list) {
          var x = list.filter(function (y) { return y.id === pick.value; })[0];
          if (x && x.file) { player.src = R.fileUrl(x.file); player.hidden = false; } else player.hidden = true;
        });
      });
      ms.appendChild(h('div', { class: 'row' }, pick, path));
      ms.appendChild(player);
      ms.appendChild(h('div', { class: 'row' }, R.field('Volume dB', vol), R.field('Fade in', fi), R.field('Fade out', fo), h('label', { class: 'chk small' }, duck, ' duck under speech'),
        R.button(mu ? 'Update music' : 'Add music', function () {
          var p = pick.value ? 'asset:' + pick.value : path.value.trim().replace(/^"|"$/g, '');
          if (!p) { if (mu) p = mu.path; else { R.toast('Pick a library track or enter a file path', 'bad'); return; } }
          return E.mutate('audio.add_music', { path: p, volume_db: Number(vol.value), duck: duck.checked, fade_in: Number(fi.value), fade_out: Number(fo.value) }, { toast: 'Music set' });
        }, { small: true, primary: true })));
    }

    // sfx
    var sx = sec('Sound effects', h('span', { class: 'muted small' }, 'markers on A1 · drag to retime'));
    el.appendChild(sx);
    var list = a.sfx || [];
    if (!list.length) sx.appendChild(h('div', { class: 'muted small' }, 'No sound effects yet.'));
    list.forEach(function (s) {
      var t = s.resolved_at;
      var at = num(t !== null && t !== undefined ? R.round(t, 2) : '', 0.01, 0, undefined, 'num short');
      sx.appendChild(h('div', { class: 'sfxrow' + (E.selSfx === s.id ? ' sel' : ''), 'data-sfx': s.id },
        h('b', { class: 'mono small' }, s.id), h('span', { class: 'small ell', title: s.file }, base(s.file)),
        h('span', { class: 'muted small' }, (s.volume_db || 0) + ' dB'),
        s.at_word ? h('span', { class: 'badge b-info', title: 'follows word ' + s.at_word.word_index + ' through re-cuts' }, 'word') : null,
        s.problem ? h('span', { class: 'small st-warn', title: s.problem }, '!') : null,
        h('span', { class: 'grow' }),
        at,
        R.button('Move', function () { return P.retimeSfx(E, s, Number(at.value)); }, { small: true, title: 'audio.remove_sfx + audio.add_sfx at the new time' }),
        t !== null && t !== undefined ? h('button', { class: 'small ghost', type: 'button', title: 'go to', onclick: function () { R.monitor.seek(t + 0.001); } }, 'go') : null,
        h('button', { class: 'small ghost', type: 'button', title: 'remove', onclick: function () { E.mutate('audio.remove_sfx', { sfx_id: s.id }, { toast: 'Removed ' + s.id }); } }, '✕')));
    });
    if (R.has('audio.add_sfx')) {
      var sp = h('select', null, R.option('', 'loading library...'));
      var sv = num(-6, 0.5, -60, 12);
      var prev = h('audio', { preload: 'none' });
      P.library('sfx').then(function (lib) {
        R.clear(sp);
        if (!lib.length) sp.appendChild(R.option('', 'no sfx in the library'));
        lib.forEach(function (x) { sp.appendChild(R.option(x.id, x.name.replace(/^Kenney\s+/, ''), x.id === E.armedSfx)); });
        if (!E.armedSfx && lib[0]) E.armedSfx = lib[0].id;
      });
      sp.addEventListener('change', function () { E.armedSfx = sp.value; });
      var atLabel = h('span', { class: 'mono small' }, R.tc(E.t, E.fps));
      var offT = function (t) { atLabel.textContent = R.tc(t, E.fps); };
      E.on('time', offT);
      sx.appendChild(h('div', { class: 'row addsfx' }, sp,
        h('button', { class: 'small ghost', type: 'button', title: 'listen', onclick: function () {
          P.library('sfx').then(function (lib) { var x = lib.filter(function (y) { return y.id === sp.value; })[0]; if (x && x.file) { prev.src = R.fileUrl(x.file); prev.play().catch(function () {}); } });
        } }, '▶'),
        sv, h('span', { class: 'muted small' }, 'dB'),
        R.button('+ Add at playhead', function () {
          if (!sp.value) { R.toast('Pick a sound from the library', 'bad'); return; }
          return E.mutate('audio.add_sfx', { file: 'asset:' + sp.value, at: R.round(E.t, 3), volume_db: Number(sv.value) }, { toast: 'SFX added at ' + R.tc(E.t, E.fps) });
        }, { small: true, primary: true }), atLabel));
      sx.appendChild(h('div', { class: 'muted small' }, 'Tip: double-click the A1 track to drop the selected sound there.'));
    }
  };
  // There is no audio.edit_sfx: retiming = remove + add (same file and gain; word anchors become fixed times).
  P.retimeSfx = function (E, s, t) {
    if (!isFinite(t) || t < 0) { R.toast('Enter a time in seconds', 'bad'); return Promise.resolve(null); }
    t = R.round(Math.min(t, E.duration - 0.01), 3);
    return E.mutate('audio.remove_sfx', { sfx_id: s.id }, {}).then(function (d) {
      if (!d) return null;
      return E.mutate('audio.add_sfx', { file: s.file, at: t, volume_db: s.volume_db }, { toast: 'SFX moved to ' + R.tc(t, E.fps) });
    });
  };
  P.addArmedSfx = function (E, t) {
    if (!E.armedSfx || !R.has('audio.add_sfx')) { R.toast('Pick a sound in the Audio tab first', 'bad'); return; }
    return E.mutate('audio.add_sfx', { file: 'asset:' + E.armedSfx, at: R.round(t, 3) }, { toast: 'SFX added at ' + R.tc(t, E.fps) });
  };

  // ------------------------------------------------------------ Effects
  P.effects = function (el, E) {
    var tl = E.transitionList || {};
    // transitions
    if (R.has('transition.set')) {
      var ts = sec('Transitions', h('span', { class: 'muted small' }, 'or click a seam on V1'));
      el.appendChild(ts);
      ts.appendChild(h('div', { class: 'small' }, 'Rendered ', h('b', { class: 'mono' }, R.tc(tl.rendered_duration !== undefined ? tl.rendered_duration : E.duration, E.fps)),
        ' of ', h('span', { class: 'mono' }, R.tc(E.duration, E.fps)), (tl.removed ? ' (transitions overlap ' + R.round(tl.removed, 2) + ' s)' : '')));
      if (E.timeline.length < 2) ts.appendChild(h('div', { class: 'muted small' }, 'One segment only: split the clip (S) to get a cut to transition across.'));
      E.timeline.slice(0, -1).forEach(function (m, i) {
        ts.appendChild(P.transitionEditor(E, m.segment_id, i));
      });
      if ((E.transitions || []).length && R.has('transition.clear')) ts.appendChild(h('div', { class: 'row' }, R.button('Clear all transitions', function () { return E.mutate('transition.clear', {}, { toast: 'All cuts are hard cuts again' }); }, { small: true, cls: 'ghost danger' })));
    }
    // effects
    if (R.has('effect.add')) {
      var es = sec('Effects (FX track)');
      el.appendChild(es);
      (E.effects || []).forEach(function (f) {
        es.appendChild(h('div', { class: 'fxrow' + (E.selFx === f.id ? ' sel' : ''), 'data-fx': f.id },
          h('span', { class: 'fxk k-' + f.kind }, f.kind), h('span', { class: 'mono small' }, R.fmt(f.start) + '–' + R.fmt(f.end)),
          f.kind !== 'flash' ? h('span', { class: 'muted small' }, '× ' + R.round(f.amount_used, 3)) : null,
          f.rendered_start === null ? h('span', { class: 'small st-warn' }, 'outside the clip') : null,
          h('span', { class: 'grow' }),
          h('button', { class: 'small ghost', type: 'button', onclick: function () { R.monitor.seek(f.start + 0.001); } }, 'go'),
          R.has('effect.remove') ? h('button', { class: 'small ghost', type: 'button', title: 'remove', onclick: function () { E.mutate('effect.remove', { effect_id: f.id }, { toast: 'Removed ' + f.id }); } }, '✕') : null));
      });
      if (!(E.effects || []).length) es.appendChild(h('div', { class: 'muted small' }, 'No effects yet.'));
      var r = E.range || { start: E.t, end: Math.min(E.duration, E.t + 1.5) };
      var kind = R.select(EFFECTS, 'punch-in');
      var s0 = num(R.round(r.start, 2), 0.01, 0), s1 = num(R.round(r.end, 2), 0.01, 0);
      var amt = num('', 0.01, 0, 2);
      amt.placeholder = 'auto';
      E.onRange = function (rr) { s0.value = R.round(rr.start, 2); s1.value = R.round(rr.end, 2); };
      es.appendChild(h('div', { class: 'row addfx' }, R.field('Kind', kind), R.field('Start', s0), R.field('End', s1), R.field('Amount', amt),
        R.button('+ Add effect', function () {
          var i = { kind: kind.value, start: Number(s0.value), end: Number(s1.value) };
          if (amt.value !== '') i.amount = Number(amt.value);
          if (!(i.end > i.start)) { R.toast('End must be after start', 'bad'); return; }
          return E.mutate('effect.add', i, { toast: kind.value + ' added' });
        }, { small: true, primary: true })));
      es.appendChild(h('div', { class: 'muted small' }, 'Drag on the FX track to select a range. punch-in/zoom-slow: zoom 1.01–2 (1.15); shake: 0.002–0.08 of the frame (0.02); flash: white flash.'));
    }
    // progress bar
    if (R.has('overlay.progress_bar')) {
      var pb = E.progressBar;
      var ps = sec('Progress bar');
      el.appendChild(ps);
      var on = h('input', { type: 'checkbox', checked: !!pb });
      var col = h('input', { type: 'color', value: (pb && pb.color ? pb.color : '#FFE000').toLowerCase() });
      var pos = R.select([['top', 'top'], ['bottom', 'bottom']], pb ? pb.position : 'top');
      var hh = num(pb ? pb.height : 8, 1, 2, 200);
      ps.appendChild(h('div', { class: 'row' }, h('label', { class: 'chk' }, on, ' Show'), R.field('Colour', col), R.field('Position', pos), R.field('Height px', hh),
        R.button('Apply', function () {
          var i = on.checked ? { enabled: true, color: col.value.toUpperCase(), position: pos.value, height: Number(hh.value) || 8 } : { enabled: false };
          return E.mutate('overlay.progress_bar', i, { toast: on.checked ? 'Progress bar on' : 'Progress bar removed' });
        }, { small: true, primary: true })));
    }
  };
  // Kind + duration picker for the cut after `segId` (used by the tab and the V1 seam popover).
  P.transitionEditor = function (E, segId, i, done) {
    var cur = (E.transitions || []).filter(function (t) { return t.after_segment_id === segId; })[0];
    var kind = R.select(TRANSITIONS, cur ? cur.kind : 'dissolve');
    var dur = num(cur ? cur.duration : 0.3, 0.05, 0.05, 2);
    var m = E.tlById(segId);
    return h('div', { class: 'transrow', 'data-after': segId },
      h('span', { class: 'small' }, h('b', null, 'Seg ' + (i + 1) + ' → ' + (i + 2)), ' ', h('span', { class: 'muted mono' }, m ? 'cut at ' + R.fmt(m.out_end) : ''),
        cur && cur.overlap ? h('span', { class: 'muted', title: cur.frames + ' frames' }, ' · ' + cur.kind + ', ' + R.round(cur.overlap, 3) + ' s overlap') : (cur ? null : h('span', { class: 'muted' }, ' · hard cut')),
        cur && cur.problem ? h('span', { class: 'st-warn', title: cur.problem }, ' · ' + cur.problem) : null),
      kind, dur, h('span', { class: 'muted small' }, 's'),
      h('span', { class: 'grow' }),
      R.button(cur ? 'Update' : 'Set', function () {
        return E.mutate('transition.set', { after_segment_id: segId, kind: kind.value, duration: Number(dur.value) || 0.3 }, { toast: 'Transition ' + kind.value + ' after segment ' + (i + 1) }).then(function (d) { if (d && done) done(); });
      }, { small: true, primary: true }),
      cur && R.has('transition.clear') ? R.button('✕', function () { return E.mutate('transition.clear', { after_segment_id: segId }, { toast: 'Hard cut' }).then(function (d) { if (d && done) done(); }); }, { small: true, cls: 'ghost', title: 'remove (hard cut)' }) : null);
  };

  // ------------------------------------------------------------ QC
  // QC commands re-save clip.json (revision +1): reload quietly so live sync does not redraw the tab,
  // and keep the last results per clip so they survive tab switches.
  var qcKeep = {};
  P.qc = function (el, E) {
    var keep = qcKeep[E.clipId] = qcKeep[E.clipId] || {};
    function quiet(p) { return p.then(function (d) { return E.reload('quiet').then(function () { return d; }); }); }
    var out = keep.out || (keep.out = h('div'));
    var renders = E.clip.renders || [];
    el.appendChild(h('div', { class: 'row' },
      R.button('Run QC on latest render', function () {
        if (!renders.length) { R.toast('Render the clip first (Render tab)', 'bad'); return; }
        return quiet(R.run('qc.run', { clip_id: E.clipId, allow_fail: true })).then(function (r) { if (r) { R.clear(out); R.renderQc(out, r); fixes(out); } });
      }, { primary: true, small: true, need: ['qc.run'] }),
      R.has('fix.suggest') ? R.button('Suggest fixes', function () { R.clear(out); return fixes(out); }, { small: true }) : null,
      R.has('fix.apply') ? R.button('Apply auto-safe fixes', function () { return applyFixes(); }, { small: true, title: 'fix.apply: safe, non-creative fixes, then re-render + re-QC' }) : null));
    var rr = h('label', { class: 'chk small' }, h('input', { type: 'checkbox', checked: true }), ' re-render + re-QC after fixing');
    if (R.has('fix.apply')) el.appendChild(h('div', { class: 'row' }, rr));
    var fixOut = keep.fixOut || (keep.fixOut = h('div'));
    el.appendChild(fixOut);
    el.appendChild(out);
    var last = renders[renders.length - 1];
    if (last && last.qc && !out.firstChild) R.getJson(R.optUrl(last.qc)).then(function (rep) { if (rep && !out.firstChild) { out.appendChild(h('div', { class: 'muted small' }, 'Saved QC of ' + last.id)); R.renderQc(out, rep); } });

    function applyFixes() {
      if (!renders.length) { R.toast('Render the clip first', 'bad'); return; }
      var rerender = rr.querySelector('input').checked;
      R.clear(fixOut).appendChild(h('div', { class: 'muted small' }, 'fix.apply running' + (rerender ? ' (re-render + QC)...' : '...')));
      return R.jobs.start('fix.apply', { clip_id: E.clipId, all_safe: true, rerender: rerender }).then(function (jid) {
        if (!jid) return;
        return new Promise(function (res) {
          var off = R.jobs.on(function (list) {
            var j = list.filter(function (x) { return x.id === jid; })[0];
            if (!j) return;
            if (j.status === 'running') { var p = j.progress || {}; fixOut.firstChild.textContent = 'fix.apply: ' + (p.stage || 'working') + (p.pct !== null && p.pct !== undefined ? ' ' + Math.round(p.pct * 100) + '%' : ''); return; }
            off();
            R.clear(fixOut);
            if (j.status === 'ok' && j.result && j.result.data) fixOut.appendChild(applyView(j.result.data));
            else fixOut.appendChild(h('div', { class: 'err small' }, R.errText('fix.apply', j.result)));
            E.reload('mutate');
            res();
          });
        });
      });
    }
    function applyView(d) {
      var b = sec('fix.apply result');
      b.appendChild(h('div', { class: 'small' }, d.fixed_checks.length ? 'Fixed: ' + d.fixed_checks.join(', ') : 'Nothing auto-safe to fix.', ' · revision ', d.revision_before, ' → ', d.revision_after));
      (d.applied || []).forEach(function (s) { b.appendChild(h('div', { class: 'small' }, '✓ ', h('span', { class: 'mono' }, s.cli || s.command), ' — ', s.change)); });
      (d.not_applied || []).forEach(function (s) { b.appendChild(h('div', { class: 'small muted' }, '· not auto-safe: ', s.check, ' — ', s.why)); });
      if (d.after) b.appendChild(h('div', { class: 'small ' + (d.after.passed ? 'st-pass' : 'st-warn') }, 'After: ', d.after.render_id, ' · ', d.after.verdict));
      return b;
    }
    function fixes(box) {
      if (!R.has('fix.suggest')) return Promise.resolve();
      return quiet(R.run('fix.suggest', { clip_id: E.clipId }, true)).then(function (d) {
        if (!d) return;
        var items = d.suggestions || [];
        var s = sec('Suggested fixes');
        box.appendChild(s);
        if (d.verdict) s.appendChild(h('div', { class: 'small ' + (d.passed ? 'st-pass' : 'st-warn') }, d.verdict));
        if (!items.length) { s.appendChild(h('span', { class: 'muted small' }, 'No fixes suggested.')); return; }
        items.forEach(function (f) {
          s.appendChild(h('div', { class: 'fixrow' },
            h('div', { class: 'grow' },
              h('div', null, h('b', null, f.check), ' ', R.badge(f.severity || '', f.severity === 'high' ? 'fail' : 'warn'), f.auto_safe ? R.badge('auto-safe', 'pass') : null),
              h('div', { class: 'small' }, f.why || ''),
              (f.commands || []).map(function (c) { return R.cliRow(c, { mutate: E.mutate }); })),
            R.has('fix.apply') && f.auto_safe ? R.button('Apply', function () { return E.mutate('fix.apply', { check: f.check, rerender: false }, { toast: 'Fix applied: ' + f.check + ' (render again to check)' }); }, { small: true, primary: true }) : null));
        });
      });
    }

    // compare two renders
    if (R.has('qc.compare') && renders.length >= 2) {
      var cs = sec('Compare two renders (qc.compare)');
      el.appendChild(cs);
      var opts = renders.slice().reverse().map(function (r) { return [r.id, r.id + ' · ' + r.quality + ' · ' + String(r.created_at || '').replace('T', ' ').slice(5, 16)]; });
      var A = R.select(opts, renders[renders.length - 2].id), B = R.select(opts, renders[renders.length - 1].id);
      var cmp = keep.cmp || (keep.cmp = h('div'));
      cs.appendChild(h('div', { class: 'row' }, R.field('Left (A)', A), R.field('Right (B)', B),
        R.button('Compare', function () {
          if (A.value === B.value) { R.toast('Pick two different renders', 'bad'); return; }
          return quiet(R.run('qc.compare', { clip_id: E.clipId, render_a: A.value, render_b: B.value })).then(function (d) {
            if (!d) return;
            R.clear(cmp);
            cmp.appendChild(h('div', { class: 'small' }, d.summary));
            cmp.appendChild(h('div', { class: 'small muted' }, 'A: ' + d.a_verdict + ' · B: ' + d.b_verdict));
            cmp.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl compact' }, h('thead', null, h('tr', null, h('th', null, 'check'), h('th', null, 'A'), h('th', null, 'B'), h('th', null, 'change'))),
              h('tbody', null, (d.diff || []).map(function (x) {
                return h('tr', null, h('td', { class: 'mono small' }, x.name), h('td', null, R.statusBadge(x.a || '-')), h('td', null, R.statusBadge(x.b || '-')),
                  h('td', { class: 'small ' + (x.change === 'improved' ? 'st-pass' : x.change === 'regressed' ? 'st-fail' : 'muted') }, x.change));
              })))));
            cmp.appendChild(h('a', { href: R.fileUrl(d.image), target: '_blank' }, h('img', { class: 'shot', src: R.fileUrl(d.image) + '&v=' + Date.now(), alt: 'side-by-side contact sheet' })));
          });
        }, { small: true, primary: true })));
      cs.appendChild(cmp);
    } else if (R.has('qc.compare')) {
      el.appendChild(h('div', { class: 'muted small' }, 'qc.compare needs two renders of this clip.'));
    }
  };

  // ------------------------------------------------------------ Variants
  P.variants = function (el, E) {
    var cs = sec('Create variants (variant.create)');
    el.appendChild(cs);
    var styleBox = h('div', { class: 'checks' }, h('span', { class: 'muted small' }, 'loading...'));
    var layoutBox = h('div', { class: 'checks' });
    var hooks = h('textarea', { rows: 2, class: 'plain', placeholder: 'hook titles, one per line (optional)' });
    function checks(box, names, cur) {
      R.clear(box);
      names.forEach(function (n) { box.appendChild(h('label', { class: 'chk small pillchk' }, h('input', { type: 'checkbox', value: n }), ' ' + n + (n === cur ? ' (current)' : ''))); });
    }
    function picked(box) { return Array.prototype.filter.call(box.querySelectorAll('input'), function (i) { return i.checked; }).map(function (i) { return i.value; }); }
    R.loadStyleDefs().then(function (list) { checks(styleBox, list.map(function (s) { return s.name; }), E.clip.captions.preset); });
    (R.has('layout.presets') ? R.run('layout.presets', {}, true) : Promise.resolve(null)).then(function (d) {
      var names = d ? (d.built_in || []).filter(function (p) { return !(p.required_params || []).length; }).map(function (p) { return p.name; }).concat((d.custom || []).map(function (p) { return p.name; })) : ['center-crop', 'blur-fit'];
      checks(layoutBox, names, '');
    });
    var count = h('span', { class: 'muted small' });
    function upd() { var n = Math.max(1, picked(styleBox).length) * Math.max(1, picked(layoutBox).length) * Math.max(1, hooks.value.split('\n').filter(function (x) { return x.trim(); }).length); count.textContent = '→ ' + n + ' variant' + (n === 1 ? '' : 's'); }
    [styleBox, layoutBox, hooks].forEach(function (x) { x.addEventListener('change', upd); x.addEventListener('input', upd); });
    cs.appendChild(R.field('Caption styles', styleBox));
    cs.appendChild(R.field('Layouts', layoutBox));
    cs.appendChild(R.field('Hooks', hooks));
    cs.appendChild(h('div', { class: 'row' }, R.button('Create variants', function () {
      var i = { clip_id: baseId() };
      var s = picked(styleBox), l = picked(layoutBox), hk = hooks.value.split('\n').map(function (x) { return x.trim(); }).filter(Boolean);
      if (s.length) i.caption_styles = s;
      if (l.length) i.layouts = l;
      if (hk.length) i.hooks = hk;
      if (!s.length && !l.length && !hk.length) { R.toast('Pick at least one style, layout or hook', 'bad'); return; }
      return R.run('variant.create', i).then(function (d) {
        if (!d) return;
        (d.warnings || []).forEach(function (w) { R.toast(w); });
        R.toast(d.variants.length + ' variant(s) created', 'good');
        drawList();
      });
    }, { small: true, primary: true, need: ['variant.create'] }), count));

    var ls = sec('Variants of ' + (E.extras && E.extras.variant_of ? 'base ' + E.extras.variant_of : 'this clip'));
    el.appendChild(ls);
    var listBox = h('div');
    ls.appendChild(listBox);
    var res = h('div');
    el.appendChild(res);
    function baseId() { return (E.extras && E.extras.variant_of) || E.clipId; }
    var current = [];
    function drawList() {
      if (!R.has('variant.list')) return;
      R.run('variant.list', { clip_id: E.clipId }, true).then(function (d) {
        R.clear(listBox);
        current = d ? [{ clip_id: d.base, name: 'base', caption_style: '', layout: null, hook: null, renders: null, base: true }].concat(d.variants || []) : [];
        if (!d || !(d.variants || []).length) { listBox.appendChild(h('div', { class: 'muted small' }, 'No variants yet.')); return; }
        listBox.appendChild(h('table', { class: 'tbl compact vtable' }, h('thead', null, h('tr', null, h('th', null, h('input', { type: 'checkbox', checked: true, onchange: function (e) { Array.prototype.forEach.call(listBox.querySelectorAll('tbody input'), function (c) { c.checked = e.target.checked; }); } })),
          h('th', null, 'clip'), h('th', null, 'style'), h('th', null, 'layout'), h('th', null, 'hook'), h('th', null, 'renders'), h('th', null, ''))),
          h('tbody', null, current.map(function (v) {
            return h('tr', { class: v.clip_id === E.clipId ? 'sel' : '' }, h('td', null, h('input', { type: 'checkbox', checked: true, value: v.clip_id })),
              h('td', { class: 'small' }, h('b', null, v.base ? 'base' : v.name)), h('td', { class: 'small' }, v.caption_style || ''), h('td', { class: 'small' }, v.layout || ''),
              h('td', { class: 'small ell' }, v.hook || ''), h('td', { class: 'mono small' }, v.renders === null ? '' : String(v.renders)),
              h('td', null, v.clip_id !== E.clipId ? h('button', { class: 'small ghost', type: 'button', onclick: function () { R.go('editor', v.clip_id); } }, 'open') : h('span', { class: 'muted small' }, 'here')));
          }))));
      });
    }
    function sel() { return Array.prototype.filter.call(listBox.querySelectorAll('tbody input'), function (c) { return c.checked; }).map(function (c) { return c.value; }); }
    var q = R.select([['draft', 'draft'], ['final', 'final']], 'draft');
    var onlyMissing = h('input', { type: 'checkbox', checked: true });
    var t = num(1.0, 0.1, 0, undefined, 'num short');
    ls.appendChild(h('div', { class: 'row' },
      R.button('Compare grid', function () {
        var ids = sel();
        if (ids.length < 2) { R.toast('Select at least two clips', 'bad'); return; }
        return R.run('variant.compare', { clip_ids: ids.slice(0, 12), out_t: Number(t.value) || 1 }).then(function (d) {
          if (!d) return;
          R.clear(res).appendChild(sec('Compare at ' + R.fmt(d.out_t)));
          res.appendChild(h('a', { href: R.fileUrl(d.image), target: '_blank' }, h('img', { class: 'shot', src: R.fileUrl(d.image) + '&v=' + Date.now(), alt: 'variant compare grid' })));
        });
      }, { small: true, need: ['variant.compare'] }), h('span', { class: 'muted small' }, 'at'), t, h('span', { class: 'muted small' }, 's')));
    ls.appendChild(h('div', { class: 'row' }, R.field('Quality', q), h('label', { class: 'chk small' }, onlyMissing, ' only missing'),
      R.button('Render selected (render.batch)', function () {
        var ids = sel();
        if (!ids.length) { R.toast('Select clips', 'bad'); return; }
        R.clear(res).appendChild(h('div', { class: 'muted small batchprog' }, 'render.batch starting...'));
        return R.jobs.start('render.batch', { clip_ids: ids, quality: q.value, only_missing: onlyMissing.checked }).then(function (jid) {
          if (!jid) return;
          var off = R.jobs.on(function (list) {
            var j = list.filter(function (x) { return x.id === jid; })[0];
            if (!j) return;
            var p = j.progress || {};
            if (j.status === 'running') { var b = res.querySelector('.batchprog'); if (b) b.textContent = 'render.batch: ' + (p.stage || '') + ' ' + (p.message || '') + (p.pct !== null && p.pct !== undefined ? ' ' + Math.round(p.pct * 100) + '%' : ''); return; }
            off();
            R.clear(res);
            if (j.status !== 'ok') { res.appendChild(h('div', { class: 'err small' }, R.errText('render.batch', j.result))); return; }
            res.appendChild(batchView(j.result.data));
            drawList();
            E.reload('render');
          });
          R.onLeave(off);
        });
      }, { small: true, primary: true, need: ['render.batch'] }),
      R.button('Export selected (export.batch)', function () {
        var ids = sel();
        if (!ids.length) { R.toast('Select clips', 'bad'); return; }
        return R.run('export.batch', { clip_ids: ids }).then(function (d) { if (d) R.clear(res).appendChild(exportView(d)); });
      }, { small: true, need: ['export.batch'] })));
    function batchView(d) {
      var b = sec('render.batch: ' + d.rendered + ' rendered · ' + d.skipped + ' skipped · ' + d.failed + ' failed (' + d.encoder + ', ' + d.parallel + ' at once)');
      b.appendChild(h('table', { class: 'tbl compact' }, h('tbody', null, (d.results || []).map(function (r) {
        return h('tr', null, h('td', { class: 'small' }, r.name), h('td', null, R.statusBadge(r.status)), h('td', { class: 'mono small' }, r.render_id || ''),
          h('td', { class: 'small muted' }, r.reason || (r.error && r.error.message) || (r.duration ? R.fmt(r.duration) : '')));
      }))));
      return b;
    }
    function exportView(d) {
      var b = sec('export.batch: ' + d.exported.length + ' exported · ' + d.skipped.length + ' skipped');
      d.exported.forEach(function (x) { b.appendChild(h('div', { class: 'small' }, R.badge('exported', 'pass'), ' ', h('b', null, x.name), ' ', h('span', { class: 'mono muted' }, base(x.video)), x.brief ? h('span', null, ' · brief ', R.statusBadge(x.brief)) : null)); });
      d.skipped.forEach(function (x) {
        b.appendChild(h('div', { class: 'skiprow' }, h('div', { class: 'small' }, R.badge('skipped', 'warn'), ' ', h('b', null, x.name), ': ', x.reasons.join('; ')),
          (x.fix || []).map(function (f) { return R.cliRow(f, {}); })));
      });
      (d.warnings || []).forEach(function (w) { b.appendChild(h('div', { class: 'small st-warn' }, w)); });
      return b;
    }
    drawList();
  };
})();
