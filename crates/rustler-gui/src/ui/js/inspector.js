// Inspector side panel: Clip, Layout, Captions, Overlays, Audio, Effects, QC, Render, Brief,
// Variants, History. Audio/Effects/QC/Variants live in panels.js, Brief in brief.js.
// Tabs whose commands are not registered are hidden (feature detection).
(function () {
  'use strict';
  var R = window.R, h = R.h;
  var I = R.inspector = {};
  var E = null, tabs = null, host = null;

  I.mount = function (el, state) {
    E = state; host = el; tabs = null; cueBox = null;
    E.on('load', function (reason) {
      if (!tabs) build();
      else if (reason !== 'render' || tabs.current() === 'render') tabs.refresh();
    });
    E.on('select', function () { if (tabs && tabs.current() === 'layout') tabs.refresh(); });
    E.on('captions', function () { if (tabs && tabs.current() === 'captions') drawCueList(); });
  };
  I.showTab = function (id) { if (tabs) tabs.show(id); };

  function build() {
    var items = [
      { id: 'clip', label: 'Clip', render: clipTab },
      { id: 'layout', label: 'Layout', render: layoutTab },
      { id: 'captions', label: 'Captions', render: captionsTab, need: ['caption.set', 'clip.set_captions'] },
      { id: 'overlays', label: 'Overlays', render: overlaysTab, need: R.cmdsWithPrefix('overlay.') },
      { id: 'audio', label: 'Audio', render: function (el) { R.panels.audio(el, E); }, need: ['audio.add_music', 'audio.remove_music'].concat(R.cmdsWithPrefix('audio.')) },
      { id: 'effects', label: 'Effects', render: function (el) { R.panels.effects(el, E); }, need: R.cmdsWithPrefix('effect.').concat(R.cmdsWithPrefix('transition.')) },
      { id: 'qc', label: 'QC', render: function (el) { R.panels.qc(el, E); }, need: ['qc.run'] },
      { id: 'render', label: 'Render', render: renderTab, need: ['render.run'] },
      { id: 'brief', label: 'Brief', render: function (el) { R.briefPanel(el, E.clipId, E.mutate); }, need: R.cmdsWithPrefix('brief.') },
      { id: 'variants', label: 'Variants', render: function (el) { R.panels.variants(el, E); }, need: R.cmdsWithPrefix('variant.') },
      { id: 'history', label: 'History', render: historyTab, need: ['history.list'] }
    ].filter(function (t) { return !t.need || R.hasAny(t.need); });
    tabs = R.tabs(items, 'inspector', 'insp');
    host.appendChild(tabs);
  }
  function sec(title, extra) { return h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, title), extra || null)); }

  // ------------------------------------------------------------ Clip
  function clipTab(el) {
    var c = E.clip;
    el.appendChild(h('div', { class: 'kv' },
      h('span', null, 'Name'), h('b', null, c.name),
      h('span', null, 'Duration'), h('b', { class: 'mono' }, R.tc(E.duration, E.fps) + '  (' + E.duration.toFixed(2) + ' s)'),
      h('span', null, 'Canvas'), h('b', { class: 'mono' }, c.canvas.width + 'x' + c.canvas.height + ' @ ' + c.canvas.fps),
      h('span', null, 'Revision'), h('b', { class: 'mono' }, String(E.rev))));
    var s = sec('Segments');
    var tb = h('tbody');
    E.timeline.forEach(function (m, i) {
      var seg = E.segById(m.segment_id);
      var tr = h('tr', { class: m.segment_id === E.selSeg ? 'sel' : '', onclick: function () { E.selectSeg(m.segment_id, true); } },
        h('td', null, String(i + 1)), h('td', { class: 'mono' }, R.fmt(m.src_start) + '–' + R.fmt(m.src_end)),
        h('td', { class: 'mono' }, R.fmt(m.out_start) + '–' + R.fmt(m.out_end)), h('td', null, (seg && seg.layout.preset) || 'custom'),
        h('td', null, i > 0 ? h('button', { class: 'small ghost', type: 'button', title: 'move left', onclick: function (e) { e.stopPropagation(); E.mutate('edit.move_segment', { segment_id: m.segment_id, to_index: i - 1 }); } }, '↑') : null,
          E.timeline.length > 1 ? h('button', { class: 'small ghost', type: 'button', title: 'delete', onclick: function (e) { e.stopPropagation(); E.mutate('edit.delete_segment', { segment_id: m.segment_id }); } }, '✕') : null));
      tb.appendChild(tr);
    });
    s.appendChild(h('table', { class: 'tbl compact' }, h('thead', null, h('tr', null, h('th', null, '#'), h('th', null, 'source'), h('th', null, 'output'), h('th', null, 'layout'), h('th', null, ''))), tb));
    el.appendChild(s);

    // Set range by transcript units (selected segment)
    var sr = sec('Set range by transcript units');
    el.appendChild(sr);
    var seg = E.segById(E.selSeg);
    if (!seg || !R.has('edit.set_range')) sr.appendChild(R.na('edit.set_range'));
    else unitPicker(sr, seg);

    var cl = sec('Clean up');
    var gap = h('input', { type: 'number', step: '0.05', min: 0.1, value: 0.5, class: 'num' });
    var words = h('input', { placeholder: 'um, uh (blank = default fillers)' });
    cl.appendChild(h('div', { class: 'row' }, R.field('min pause s', gap),
      R.button('Remove pauses', function () { return E.mutate('edit.remove_pauses', { min_gap: Number(gap.value) || 0.5, keep: 0.12 }, { toast: 'Pauses removed' }); }, { small: true, need: ['edit.remove_pauses'] })));
    cl.appendChild(h('div', { class: 'row' }, words,
      R.button('Remove words', function () {
        var list = words.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
        return E.mutate('edit.remove_words', list.length ? { words: list } : {}, { toast: 'Words removed' });
      }, { small: true, need: ['edit.remove_words'] })));
    if (R.has('edit.refine')) cl.appendChild(h('div', { class: 'row' }, R.button('Refine cut points', function () { return E.mutate('edit.refine', {}, { toast: 'Cut refined' }); }, { small: true }), h('span', { class: 'muted small' }, R.spec('edit.refine').summary)));
    el.appendChild(cl);
  }
  function unitPicker(box, seg) {
    var sid = seg.source_id;
    var list = h('div', { class: 'units small-units' }, h('span', { class: 'muted small' }, 'loading units...'));
    var pick = { s: null, e: null };
    var status = h('span', { class: 'small' }, 'click a start unit, then an end unit');
    var apply = R.button('Apply range', function () {
      if (pick.s === null || pick.e === null) { R.toast('Pick a start and an end unit', 'bad'); return; }
      return E.mutate('edit.set_range', { segment_id: seg.id, start_unit: pick.s.id, end_unit: pick.e.id }, { toast: 'Range set' });
    }, { small: true, primary: true });
    box.appendChild(h('div', { class: 'row' }, status, h('span', { class: 'grow' }), apply));
    box.appendChild(list);
    // units around the segment (2 minutes either side), full objects
    var key = sid + '@' + Math.floor(seg.start / 60);
    var cache = E.units[key];
    var p = cache ? Promise.resolve(cache) : R.run('transcript.get', { source_id: sid, format: 'json', start: Math.max(0, seg.start - 120), end: seg.end + 120, limit: 400 }, true)
      .then(function (d) { var u = d ? R.extractUnits(d) : []; E.units[key] = u; return u; });
    p.then(function (units) {
      R.clear(list);
      if (!units.length) { list.appendChild(h('span', { class: 'muted small' }, 'no transcript units')); return; }
      units.forEach(function (u) {
        var inSeg = u.end > seg.start && u.start < seg.end;
        var row = h('div', { class: 'unit' + (inSeg ? ' in' : '') }, h('span', { class: 't' }, '#' + u.id), h('span', { class: 't' }, R.fmt(u.start)), h('span', { class: 'x' }, u.text));
        row.addEventListener('click', function () {
          if (pick.s === null || pick.e !== null) { pick.s = u; pick.e = null; } else { pick.e = u.id >= pick.s.id ? u : pick.s; if (u.id < pick.s.id) pick.s = u; }
          Array.prototype.forEach.call(list.children, function (r) { r.classList.remove('s', 'e'); });
          row.classList.add(pick.e ? 'e' : 's');
          status.textContent = 'start #' + pick.s.id + (pick.e ? '  end #' + pick.e.id : '  (now pick the end)');
        });
        list.appendChild(row);
      });
    });
  }

  // ------------------------------------------------------------ Layout
  function layoutTab(el) {
    var seg = E.segById(E.selSeg);
    if (!seg) { el.appendChild(h('p', { class: 'muted' }, 'No segment.')); return; }
    var lay = seg.layout;
    var scopeTxt = E.scopeAll && E.timeline.length > 1 ? 'all segments' : 'segment ' + (E.timeline.indexOf(E.tlById(seg.id)) + 1);
    el.appendChild(h('div', { class: 'muted small' }, 'Editing: ', h('b', null, scopeTxt), '  ·  preset ', h('b', null, lay.preset || 'custom'), '  ·  background ', h('b', null, lay.background)));
    // presets
    var ps = sec('Presets');
    el.appendChild(ps);
    if (R.has('layout.presets')) {
      R.run('layout.presets', {}, true).then(function (d) {
        if (!d) return;
        var built = (d.built_in || []), custom = (d.custom || []);
        var opts = built.filter(function (p) { return !(p.required_params || []).length; }).map(function (p) { return ['b:' + p.name, p.name]; })
          .concat(custom.map(function (p) { return ['c:' + p.name, p.name + ' (custom)']; }));
        var sel = R.select(opts, lay.preset ? (custom.some(function (p) { return p.name === lay.preset; }) ? 'c:' : 'b:') + lay.preset : '');
        var desc = h('div', { class: 'muted small' });
        var setDesc = function () { var v = sel.value.slice(2); var p = built.concat(custom).filter(function (x) { return x.name === v; })[0]; desc.textContent = p ? (p.description || '') : ''; };
        sel.addEventListener('change', setDesc); setDesc();
        ps.appendChild(h('div', { class: 'row' }, sel, R.button('Apply', function () {
          var input = { name: sel.value.slice(2) };
          if (!E.scopeAll) input.segment_id = seg.id;
          return E.mutate('layout.apply_preset', input, { toast: 'Preset applied' });
        }, { small: true, primary: true, need: ['layout.apply_preset'] })));
        ps.appendChild(desc);
        if (custom.length && R.has('layout.delete_preset')) {
          ps.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted small' }, 'custom: '), custom.map(function (p) {
            return h('span', { class: 'pill' }, p.name, h('button', { class: 'x', type: 'button', title: 'delete preset', onclick: function () {
              if (!confirm('Delete custom preset ' + p.name + '?')) return;
              R.run('layout.delete_preset', { name: p.name }).then(function (r) { if (r) { R.toast('Deleted', 'good'); tabs.refresh(); } });
            } }, '✕'));
          })));
        }
        var nm = h('input', { placeholder: 'new preset name', class: 'grow' });
        ps.appendChild(h('div', { class: 'row' }, nm, R.button('Save as preset', function () {
          if (!nm.value.trim()) { R.toast('Name the preset', 'bad'); return; }
          return R.run('layout.save_preset', { name: nm.value.trim(), from_clip_id: E.clipId, segment_id: seg.id, overwrite: true }).then(function (r) { if (r) { R.toast('Preset saved: ' + r.name, 'good'); tabs.refresh(); } });
        }, { small: true, need: ['layout.save_preset'] })));
      });
    } else ps.appendChild(R.na('layout.presets'));
    ps.appendChild(h('div', { class: 'row' },
      R.button('Detect webcam', function () { return R.regions.detectWebcam(); }, { small: true, need: ['layout.detect_webcam'] }),
      R.button('Streamer split (draw 2 boxes)', function () { return R.regions.streamerSplit(); }, { small: true, need: ['layout.from_frame_boxes'] }),
      R.button('+ Region', function () { return R.regions.addRegion(); }, { small: true, need: ['layout.add_region'] }),
      R.has('crop.track_speaker') ? R.button('Track active speaker', function () {
        return R.jobs.start('crop.track_speaker', { clip_id: E.clipId }).then(function (jid) {
          if (!jid) return;
          R.toast('Tracking the active speaker...');
          var id = E.clipId;
          var off = R.jobs.on(function (list) {
            var j = list.filter(function (x) { return x.id === jid; })[0];
            if (!j || j.status === 'running') return;
            off();
            if (j.status === 'ok' && j.result && j.result.data) {
              var d = j.result.data;
              (d.warnings || []).forEach(function (w) { R.toast(w); });
              R.toast('Crop follows the active speaker in ' + (d.segments || []).length + ' segment(s)', 'good');
            }
            if (E && E.clipId === id) E.reload('mutate');
          });
        });
      }, { small: true, title: 'crop.track_speaker: center-crop follows whoever is talking (needs analyze.speakers)' }) : null));

    // regions
    var rs = sec('Regions (back to front)');
    el.appendChild(rs);
    var regs = lay.regions.slice().map(function (r, ix) { return Object.assign({ ix: ix }, r); }).sort(function (a, b) { return a.z - b.z; });
    regs.forEach(function (r, k) {
      var selected = r.id === E.selRegion;
      var row = h('div', { class: 'region-row' + (selected ? ' sel' : ''), onclick: function () { E.selectRegion(r.id); } },
        h('i', { class: 'swatch', style: { background: R.regions.color(r.ix) } }),
        h('b', null, r.id), h('span', { class: 'muted small' }, r.fit + ' · z ' + r.z),
        h('span', { class: 'grow' }),
        k > 0 ? h('button', { class: 'small ghost', type: 'button', title: 'send backward', onclick: function (e) { e.stopPropagation(); reorder(regs, k, k - 1, seg); } }, '↓') : null,
        k < regs.length - 1 ? h('button', { class: 'small ghost', type: 'button', title: 'bring forward', onclick: function (e) { e.stopPropagation(); reorder(regs, k, k + 1, seg); } }, '↑') : null,
        regs.length > 1 ? h('button', { class: 'small ghost', type: 'button', title: 'remove region', onclick: function (e) {
          e.stopPropagation();
          var input = { region_id: r.id }; if (!E.scopeAll) input.segment_id = seg.id;
          E.mutate('layout.remove_region', input, { toast: 'Region removed' });
        } }, '✕') : null);
      rs.appendChild(row);
      if (selected) rs.appendChild(regionEditor(r, seg));
    });
    if (!regs.length) rs.appendChild(h('p', { class: 'muted small' }, 'No regions.'));
  }
  function reorder(regs, from, to, seg) {
    var ids = regs.map(function (r) { return r.id; });
    var x = ids.splice(from, 1)[0]; ids.splice(to, 0, x);
    var input = { region_ids: ids }; if (!E.scopeAll) input.segment_id = seg.id;
    return E.mutate('layout.reorder', input);
  }
  function regionEditor(r, seg) {
    var box = h('div', { class: 'region-ed' });
    var ins = {};
    function grp(which, rect) {
      var row = h('div', { class: 'rgrid' }, h('span', { class: 'muted small' }, which === 'src' ? 'source %' : 'canvas %'));
      ['x', 'y', 'w', 'h'].forEach(function (k) {
        var inp = h('input', { type: 'number', step: '0.1', min: 0, max: 100, value: R.round(rect[k] * 100, 1), title: which + '.' + k + ' (% of the ' + (which === 'src' ? 'source frame' : 'canvas') + ')' });
        ins[which + k] = inp;
        row.appendChild(h('label', { class: 'mini' }, h('span', null, k), inp));
      });
      return row;
    }
    var fit = R.select([['cover', 'cover (fill + crop)'], ['contain', 'contain (fit inside)']], r.fit);
    box.appendChild(grp('src', r.src));
    box.appendChild(grp('dst', r.dst));
    box.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted small' }, 'fit'), fit, h('span', { class: 'grow' }),
      R.button('Apply', function () {
        var get = function (w) { return { x: Number(ins[w + 'x'].value) / 100, y: Number(ins[w + 'y'].value) / 100, w: Number(ins[w + 'w'].value) / 100, h: Number(ins[w + 'h'].value) / 100 }; };
        var input = { region_id: r.id, src: get('src'), dst: get('dst'), fit: fit.value };
        if (!E.scopeAll) input.segment_id = seg.id;
        return E.mutate('layout.edit_region', input, { toast: 'Region updated' });
      }, { small: true, primary: true, need: ['layout.edit_region'] })));
    box.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted small' }, 'quick:'),
      R.button('Fill canvas', function () { return edit(r, seg, { dst: { x: 0, y: 0, w: 1, h: 1 } }); }, { small: true }),
      R.button('Top half', function () { return edit(r, seg, { dst: { x: 0, y: 0, w: 1, h: 0.5 } }); }, { small: true }),
      R.button('Bottom half', function () { return edit(r, seg, { dst: { x: 0, y: 0.5, w: 1, h: 0.5 } }); }, { small: true }),
      R.button('Whole source', function () { return edit(r, seg, { src: { x: 0, y: 0, w: 1, h: 1 } }); }, { small: true })));
    return box;
  }
  function edit(r, seg, patch) {
    var input = Object.assign({ region_id: r.id }, patch);
    if (!E.scopeAll) input.segment_id = seg.id;
    return E.mutate('layout.edit_region', input);
  }

  // ------------------------------------------------------------ Captions
  var cueBox = null;
  function captionsTab(el) {
    var cap = E.clip.captions;
    var cmd = R.has('caption.set') ? 'caption.set' : 'clip.set_captions';
    var on = h('input', { type: 'checkbox', checked: cap.enabled });
    var pos = h('input', { type: 'range', min: 0.05, max: 0.95, step: 0.01, value: cap.position_y });
    var posV = h('span', { class: 'mono small' }, cap.position_y.toFixed(2));
    pos.addEventListener('input', function () { posV.textContent = Number(pos.value).toFixed(2); E.clip.captions.position_y = Number(pos.value); R.monitor.draw(); });
    var wpl = h('input', { type: 'number', min: 1, max: 12, value: cap.max_words_per_line, class: 'num' });
    var script = R.select(['latin', 'devanagari', 'as_transcribed'], cap.script);
    var styleSel = { value: cap.preset };
    var grid = h('div', { class: 'stylegrid' }, h('span', { class: 'muted small' }, 'loading styles...'));
    var prev = h('div', { class: 'capprev' });
    function save(extra) {
      var input = Object.assign({ enabled: on.checked, style: styleSel.value, position_y: Number(pos.value), max_words_per_line: Number(wpl.value) || cap.max_words_per_line, script: script.value }, extra || {});
      if (cmd === 'clip.set_captions') { input.preset = input.style; delete input.style; }
      return E.mutate(cmd, input, { toast: 'Captions updated' });
    }
    el.appendChild(h('div', { class: 'row' }, h('label', { class: 'chk' }, on, ' Captions on'), h('span', { class: 'grow' }),
      R.button('Apply', function () { return save(); }, { small: true, primary: true }),
      R.button('Exact frame', function () { return exactInto(prev); }, { small: true, need: ['render.preview_frame'] })));
    el.appendChild(h('div', { class: 'row' }, R.field('Position (0 top .. 1 bottom)', h('div', { class: 'row tight' }, pos, posV)), R.field('Words / line', wpl), R.field('Script', script)));
    el.appendChild(sec('Style'));
    el.appendChild(grid);
    el.appendChild(prev);
    drawExact(prev);
    (R.has('caption.styles') ? R.loadStyleDefs() : R.loadStyles()).then(function (styles) {
      R.clear(grid);
      styles.forEach(function (st) {
        var custom = st.built_in === false;
        var nm = h('span', { class: 'sname s-' + st.name }, sample(st.name));
        if (custom) {
          nm.style.color = st.primary || '#fff';
          nm.style.fontFamily = /anton/i.test(st.font_family || '') ? R.fonts.anton : R.fonts.poppins;
          nm.textContent = st.uppercase ? 'ASK' : 'Ask';
          nm.appendChild(h('span', { style: { color: st.active_colour } }, st.uppercase ? ' NOT' : ' not'));
        }
        var card = h('button', { type: 'button', class: 'stylecard' + (st.name === cap.preset ? ' sel' : '') + (custom ? ' custom' : ''), title: (st.description || st.name) + (custom ? ' (custom, ' + st.scope + ')' : '') },
          nm, h('span', { class: 'small' }, st.name), custom ? h('span', { class: 'cbadge' }, st.scope) : null);
        card.addEventListener('click', function () {
          styleSel.value = st.name;
          Array.prototype.forEach.call(grid.children, function (c) { c.classList.remove('sel'); });
          card.classList.add('sel');
          save({ style: st.name }).then(function (d) { if (d) return exactInto(prev); });
        });
        grid.appendChild(card);
      });
    });
    if (R.has('caption.create_style')) {
      el.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted small grow' }, 'Custom styles are saved to the asset library and work everywhere a style name does.'),
        R.button('+ New style…', function () {
          R.modal('New caption style (caption.create_style)', R.styleCreator({ base: cap.preset, editor: E, onSaved: function () { R.loadStyleDefs(); } }));
        }, { small: true, primary: true })));
    }
    if (R.has('caption.speaker_colors')) el.appendChild(speakerColors());
    var cs = sec('Caption chunks (from caption.export_srt)');
    cueBox = h('div', { class: 'cues' });
    cs.appendChild(cueBox);
    el.appendChild(cs);
    drawCueList();
  }
  // Speaker-coloured captions (caption.speaker_colors; needs analyze.speakers on the source).
  function speakerColors() {
    var box = sec('Speaker colours');
    var cur = (E.extras && E.extras.speaker_colors) || { enabled: false, colors: {} };
    var on = h('input', { type: 'checkbox', checked: !!cur.enabled });
    var pickers = h('div', { class: 'row' });
    var DEF = ['#FFE000', '#5EE0FF', '#FF7AB6', '#7CFF8A', '#FFA24D', '#B18CFF'];
    var sids = {};
    E.clip.segments.forEach(function (sg) { sids[sg.source_id] = 1; });
    var ids = {}, names = {};
    Promise.all(Object.keys(sids).map(function (sid) { return R.getJson(R.optUrl('analysis/' + sid + '/speakers.json')); })).then(function (files) {
      files.forEach(function (f) { if (f && f.segments) { f.segments.forEach(function (t) { ids[t.speaker] = 1; }); Object.keys(f.labels || {}).forEach(function (k) { names[k] = f.labels[k]; }); } });
      var list = Object.keys(ids).sort();
      if (!list.length) { pickers.appendChild(h('span', { class: 'muted small' }, 'No speaker analysis for this clip’s source yet (Sources → Understand → Analyze speakers).')); return; }
      list.forEach(function (k, i) {
        var c = h('input', { type: 'color', value: String((cur.colors || {})[k] || DEF[i % DEF.length]).toLowerCase(), 'data-speaker': k });
        pickers.appendChild(h('label', { class: 'small chk' }, c, ' ' + (names[k] ? names[k] + ' (' + k + ')' : k)));
      });
    });
    box.appendChild(h('div', { class: 'row' }, h('label', { class: 'chk' }, on, ' Colour captions by speaker'), h('span', { class: 'grow' }),
      R.button('Apply', function () {
        var colors = {};
        Array.prototype.forEach.call(pickers.querySelectorAll('input[type=color]'), function (c) { colors[c.getAttribute('data-speaker')] = c.value.toUpperCase(); });
        var input = { enabled: on.checked };
        if (Object.keys(colors).length) input.colors = colors;
        return E.mutate('caption.speaker_colors', input, { toast: on.checked ? 'Speaker colours on' : 'Speaker colours off' });
      }, { small: true, primary: true })));
    box.appendChild(pickers);
    return box;
  }
  function sample(name) { return /pop|bold|hormozi/.test(name) ? 'ASK NOT' : 'Ask not'; }
  function drawCueList() {
    if (!cueBox) return;
    R.clear(cueBox);
    if (!E.captions.length) { cueBox.appendChild(h('span', { class: 'muted small' }, E.clip.captions.enabled ? 'no chunks' : 'captions are off')); return; }
    E.captions.forEach(function (c) {
      cueBox.appendChild(h('div', { class: 'cuerow', onclick: function () { R.monitor.seek(c.start + 0.001); } }, h('span', { class: 'mono small muted' }, R.tc(c.start, E.fps)), h('span', null, c.text)));
    });
  }
  // The tab re-renders after every save, so the last exact frame is kept and redrawn.
  var lastExact = null;
  function drawExact(box) {
    if (!box || !lastExact || lastExact.clip !== E.clipId) return;
    R.clear(box).appendChild(h('div', { class: 'muted small' }, 'Exact frame at ' + R.tc(lastExact.t, E.fps) + ' · ' + lastExact.style + ' (libass, the real renderer)'));
    box.appendChild(h('img', { class: 'shot portrait', src: R.fileUrl(lastExact.image) + '&v=' + lastExact.v, alt: 'exact frame' }));
  }
  function exactInto() {
    var t = R.round(E.t, 3);
    return R.run('render.preview_frame', { clip_id: E.clipId, out_t: t }).then(function (d) {
      if (!d) return;
      lastExact = { clip: E.clipId, t: t, image: d.image, v: Date.now(), style: E.clip.captions.preset };
      drawExact(host.querySelector('.capprev'));
    });
  }

  // ------------------------------------------------------------ generic command panels
  // Lists items from `listCmd`, shows a form for every other command in `cmds`.
  function genericPanel(el, cmds, listCmd, opts) {
    var o = opts || {};
    if (listCmd && R.has(listCmd)) {
      var lb = sec(o.listTitle || 'Current');
      el.appendChild(lb);
      R.run(listCmd, { clip_id: E.clipId }, true).then(function (d) {
        if (!d) { lb.appendChild(h('span', { class: 'muted small' }, 'nothing yet')); return; }
        var items = R.listOf(d, o.listKey || 'items');
        if (!items.length) { lb.appendChild(h('span', { class: 'muted small' }, 'none yet')); return; }
        items.forEach(function (it) { lb.appendChild(o.item ? o.item(it) : h('pre', null, R.pretty(it))); });
      });
    }
    cmds.filter(function (c) { return c !== listCmd && R.has(c); }).forEach(function (c) {
      var spec = R.spec(c);
      var fixed = {};
      if (R.hasProp(c, 'clip_id')) fixed.clip_id = E.clipId;
      // time fields default to the playhead (start) and +3 s (end)
      var initial = {};
      if (R.hasProp(c, 'start') && R.schemaKind(c, 'start') && /number/.test(R.schemaKind(c, 'start').kind)) initial.start = R.round(E.t, 2);
      if (R.hasProp(c, 'end') && R.schemaKind(c, 'end') && /number/.test(R.schemaKind(c, 'end').kind) && /add|create/.test(c)) initial.end = R.round(Math.min(E.duration, E.t + 3), 2);
      if (!/add|create/.test(c)) initial = {};
      var f = R.schemaForm(c, { fixed: fixed, initial: initial, submit: c.split('.')[1].replace(/_/g, ' '), compact: true,
        run: function (cmd, input) { return (spec.annotations && spec.annotations.read_only) ? R.run(cmd, input) : E.mutate(cmd, input, { toast: cmd + ' done' }); },
        done: function (d) { if (o.onResult) o.onResult(c, d); } });
      el.appendChild(h('details', { class: 'cmdbox', open: o.open && o.open.indexOf(c) >= 0 }, h('summary', null, h('span', { class: 'mono' }, c), h('span', { class: 'muted small' }, spec.summary || '')), f.el));
    });
  }

  function overlaysTab(el) {
    var cmds = R.cmdsWithPrefix('overlay.');
    if (!cmds.length) { el.appendChild(R.na('overlay.add_text')); return; }
    genericPanel(el, cmds.filter(function (c) { return c !== 'overlay.edit' && c !== 'overlay.remove'; }), 'overlay.list', {
      listTitle: 'Overlays', listKey: 'overlays', open: ['overlay.add_text'],
      item: function (o) {
        var tm = R.overlayTimes(o);
        var id = R.overlayId(o);
        var removeCmd = R.has('overlay.remove') ? 'overlay.remove' : null;
        return h('div', { class: 'region-row' }, h('b', null, R.short(o.text || id || 'overlay', 40)),
          h('span', { class: 'muted small mono' }, tm ? R.fmt(tm.start) + '–' + R.fmt(tm.end) : ''),
          h('span', { class: 'grow' }),
          tm ? h('button', { class: 'small ghost', type: 'button', onclick: function () { R.monitor.seek(tm.start + 0.001); } }, 'go') : null,
          R.has('overlay.edit') ? h('button', { class: 'small ghost', type: 'button', onclick: function () {
            var init = {}; ['text', 'start', 'end', 'style', 'position_y', 'font_size_factor', 'box'].forEach(function (k) { if (o[k] !== undefined && o[k] !== null) init[k] = o[k]; });
            var close = null;
            var f = R.schemaForm('overlay.edit', { fixed: { clip_id: E.clipId, overlay_id: id }, initial: init, submit: 'Save overlay',
              run: function (c, input) { return E.mutate(c, input, { toast: 'Overlay saved' }); }, done: function () { if (close) close(); } });
            close = R.modal('Edit overlay ' + id, f.el);
          } }, 'edit') : null,
          removeCmd ? h('button', { class: 'small ghost', type: 'button', title: 'remove', onclick: function () {
            var key = R.hasProp(removeCmd, 'overlay_id') ? 'overlay_id' : 'id';
            var input = {}; input[key] = id;
            E.mutate(removeCmd, input, { toast: 'Overlay removed' });
          } }, '✕') : null);
      }
    });
  }

  // ------------------------------------------------------------ Render queue
  var offJobs = null;
  function renderTab(el) {
    var q = R.select([['draft', 'draft (fast)'], ['final', 'final']], R.store('rustler.quality') || 'final', function (v) { R.store('rustler.quality', v); });
    var gpu = h('input', { type: 'checkbox', checked: true });
    el.appendChild(h('div', { class: 'row' }, R.field('Quality', q), h('label', { class: 'chk small' }, gpu, ' prefer GPU'), h('span', { class: 'grow' }),
      R.button('Render', function () {
        return R.jobs.start('render.run', { clip_id: E.clipId, quality: q.value, prefer_gpu: gpu.checked }).then(function (jid) {
          if (!jid) return;
          var id = E.clipId;
          var off = R.jobs.on(function (list) { var j = list.filter(function (x) { return x.id === jid; })[0]; if (j && j.status !== 'running') { off(); if (E && E.clipId === id) E.reload('render'); } });
        });
      }, { primary: true, small: true })));
    var jb = h('div');
    el.appendChild(sec('Queue'));
    el.appendChild(jb);
    if (offJobs) offJobs();
    offJobs = R.jobs.on(function (list) {
      var mine = list.filter(function (j) { return j.input && j.input.clip_id === E.clipId && (j.command === 'render.run' || j.command === 'qc.run' || j.command === 'export.clip'); }).reverse().slice(0, 8);
      R.clear(jb);
      if (!mine.length) jb.appendChild(h('span', { class: 'muted small' }, 'No render jobs in this session.'));
      else jb.appendChild(R.jobTable(mine));
    });
    R.onLeave(function () { if (offJobs) offJobs(); offJobs = null; });
    R.jobs.poll();
    var rs = sec('Renders');
    el.appendChild(rs);
    var renders = E.clip.renders.slice().reverse();
    if (!renders.length) { rs.appendChild(h('span', { class: 'muted small' }, 'No renders yet.')); return; }
    var player = h('div', { class: 'rplayer' });
    function show(r) {
      R.clear(player);
      player.appendChild(h('video', { controls: true, preload: 'metadata', src: R.fileUrl(r.path), class: 'portrait' }));
      player.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted small mono' }, r.id + ' · ' + r.quality + ' · ' + r.encoder),
        h('span', { class: 'grow' }),
        R.button('QC', function () { R.inspector.showTab('qc'); }, { small: true, need: ['qc.run'] }),
        R.button('Export', function () {
          return R.run('export.clip', { clip_id: E.clipId, render_id: r.id, allow_fail: true }).then(function (d) { if (d) { R.toast('Exported: ' + (d.path || d.mp4 || 'done'), 'good'); player.appendChild(R.resultView(d)); } });
        }, { small: true, primary: true, need: ['export.clip'] })));
    }
    renders.forEach(function (r, i) {
      rs.appendChild(h('div', { class: 'renderrow' + (i === 0 ? ' sel' : ''), onclick: function (e) {
        Array.prototype.forEach.call(rs.querySelectorAll('.renderrow'), function (x) { x.classList.remove('sel'); });
        e.currentTarget.classList.add('sel'); show(r);
      } }, h('b', { class: 'mono small' }, r.id), h('span', { class: 'small' }, r.quality), h('span', { class: 'muted small' }, String(r.created_at || '').replace('T', ' ').slice(5, 16)),
        h('span', { class: r.qc ? 'st-pass small' : 'muted small' }, r.qc ? 'QC' : '')));
    });
    rs.appendChild(player);
    show(renders[0]);
  }

  // ------------------------------------------------------------ Brief / Variants / History
  function historyTab(el) {
    R.run('history.list', { clip_id: E.clipId }, true).then(function (d) {
      if (!d) { el.appendChild(h('span', { class: 'muted small' }, 'no history')); return; }
      var revs = (d.revisions || []).slice().reverse();
      el.appendChild(h('table', { class: 'tbl compact' }, h('thead', null, h('tr', null, h('th', null, 'rev'), h('th', null, 'time'), h('th', null, 'segs'), h('th', null, 'dur'), h('th', null, ''))),
        h('tbody', null, revs.map(function (r) {
          return h('tr', { class: r.current ? 'sel' : '' }, h('td', { class: 'mono' }, String(r.revision)), h('td', { class: 'small' }, String(r.timestamp || '').replace('T', ' ').slice(5, 19)),
            h('td', null, String(r.segments)), h('td', { class: 'mono' }, R.fmt(r.duration)),
            h('td', null, r.current ? h('span', { class: 'muted small' }, 'current') : R.button('Restore', function () { return E.mutate('history.restore', { revision: r.revision }, { toast: 'Restored revision ' + r.revision }); }, { small: true })));
        }))));
    });
  }
})();
