// Assets page: the asset library (asset.list|search|show|add|remove) with previews, "use in clip"
// actions (audio.add_sfx / audio.add_music with asset:<id>, caption.set, layout.apply_preset) and the
// custom caption style creator (caption.create_style) shared with the editor's Captions tab.
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  var KINDS = [['', 'All'], ['sfx', 'SFX'], ['music', 'Music'], ['font', 'Fonts'], ['image', 'Images'], ['caption_style', 'Caption styles'], ['layout', 'Layouts']];
  var KIND_LABEL = { sfx: 'sfx', music: 'music', font: 'font', image: 'image', caption_style: 'caption style', layout: 'layout', template: 'template' };
  var LICENSES = ['CC0-1.0', 'CC-BY-4.0', 'CC-BY-SA-4.0', 'OFL-1.1', 'own work', 'licensed: '];

  // asset.show results (preview images) by id; `v` busts the browser cache after a re-render.
  var showCache = {};
  var queue = Promise.resolve();
  function showAsset(id, force) {
    if (showCache[id] && !force) return showCache[id];
    var p = queue.then(function () { return R.run('asset.show', { id: id }, true); }).then(function (d) { if (d) d._v = Date.now(); return d; });
    queue = p.catch(function () { return null; });
    showCache[id] = p;
    return p;
  }
  R.assetShow = showAsset;

  // ------------------------------------------------------------ page
  var st = { kind: '', query: '', clip: '', at: '' };
  function render(v) {
    if (!R.hasAny(['asset.list'])) { v.appendChild(h('h1', null, 'Assets')); v.appendChild(R.na('asset.list')); return; }
    st.kind = R.store('rustler.assets.kind') || '';
    var head = h('div', { class: 'pagehead' }, h('h1', null, 'Assets'),
      h('span', { class: 'muted' }, 'Fonts, music, sound effects, images, caption styles and layouts. Every file carries its license.'),
      h('span', { class: 'grow' }),
      R.has('caption.create_style') ? R.button('+ New caption style', function () { openCreator(); }, {}) : null,
      R.button('+ Add asset', function () { addBox.hidden = !addBox.hidden; }, { primary: true, need: ['asset.add'] }));
    var addBox = addForm();
    addBox.hidden = true;
    var chips = h('div', { class: 'kindchips' });
    var search = h('input', { type: 'search', placeholder: 'search names, tags, ids (asset.search)', class: 'grow', value: st.query });
    var clipSel = h('select', { title: 'clip the "use" buttons act on' }, R.option('', 'loading clips...'));
    var at = h('input', { type: 'number', step: '0.01', min: 0, class: 'num', title: 'edit-timeline time for sound effects (defaults to the editor playhead)' });
    var useBar = h('div', { class: 'usebar' }, h('span', { class: 'muted small' }, 'Use in clip'), clipSel, h('span', { class: 'muted small' }, 'SFX at'), at, h('span', { class: 'muted small' }, 's'),
      R.button('Open in editor', function () { if (clipSel.value) R.go('editor', clipSel.value); }, { small: true, cls: 'ghost' }));
    var grid = h('div', { class: 'assetgrid' });
    var count = h('span', { class: 'muted small' });
    v.appendChild(head);
    v.appendChild(addBox);
    v.appendChild(h('div', { class: 'card assetbar' }, h('div', { class: 'row' }, chips, search), useBar));
    v.appendChild(h('div', { class: 'row' }, count));
    v.appendChild(grid);

    R.loadClips().then(function (clips) {
      R.clear(clipSel);
      if (!clips.length) { clipSel.appendChild(R.option('', 'no clips yet')); clipSel.disabled = true; return; }
      var last = R.store('rustler.lastClip');
      clips.forEach(function (c) { clipSel.appendChild(R.option(R.cid(c), c.name || R.cid(c), R.cid(c) === last)); });
      if (!clips.some(function (c) { return R.cid(c) === last; })) clipSel.value = R.cid(clips[0]);
      syncAt();
    });
    function syncAt() {
      st.clip = clipSel.value;
      var p = R.store('rustler.playhead.' + st.clip);
      at.value = p !== null && p !== undefined && p !== '' ? R.round(Number(p), 2) : 0;
    }
    clipSel.addEventListener('change', function () { R.store('rustler.lastClip', clipSel.value); syncAt(); });
    st.clipSel = clipSel; st.atEl = at;

    var all = [], presets = null, tmr = 0;
    function drawChips() {
      R.clear(chips);
      KINDS.forEach(function (k) {
        var n = k[0] ? all.filter(function (a) { return a.kind === k[0]; }).length + (k[0] === 'layout' && presets ? presets.custom.length : 0) : all.length;
        chips.appendChild(h('button', { type: 'button', class: 'kchip' + (st.kind === k[0] ? ' sel' : ''), 'data-kind': k[0] || 'all', onclick: function () {
          st.kind = k[0]; R.store('rustler.assets.kind', st.kind); drawChips(); refresh(); } }, k[1], h('span', { class: 'n' }, String(n))));
      });
    }
    function refresh() {
      var q = search.value.trim();
      st.query = q;
      var p;
      if (q && R.has('asset.search')) p = R.run('asset.search', st.kind ? { query: q, kind: st.kind } : { query: q }, true).then(function (d) { return R.listOf(d, 'hits', 'assets', 'matches'); });
      else p = Promise.resolve(all.filter(function (a) { return !st.kind || a.kind === st.kind; }));
      return p.then(function (items) {
        R.clear(grid);
        var extra = (!q && (st.kind === '' || st.kind === 'layout') && presets) ? presets.custom : [];
        count.textContent = (items.length + extra.length) + ' item' + (items.length + extra.length === 1 ? '' : 's') + (q ? ' matching "' + q + '"' : '');
        if (!items.length && !extra.length) { grid.appendChild(h('div', { class: 'card muted' }, q ? 'Nothing matches.' : 'No assets of this kind yet. Add one with "+ Add asset".')); return; }
        items.forEach(function (a) { grid.appendChild(card(a, reload)); });
        extra.forEach(function (p2) { grid.appendChild(layoutPresetCard(p2)); });
      });
    }
    function reload() {
      return Promise.all([R.run('asset.list', {}, true), R.has('layout.presets') ? R.run('layout.presets', {}, true) : null]).then(function (res) {
        all = R.listOf(res[0], 'assets');
        presets = res[1] ? { built: res[1].built_in || [], custom: res[1].custom || [] } : null;
        drawChips();
        return refresh();
      });
    }
    st.reload = reload;
    search.addEventListener('input', function () { clearTimeout(tmr); tmr = setTimeout(refresh, 250); });
    reload();
  }

  // ------------------------------------------------------------ cards
  function card(a, reload) {
    var prev = h('div', { class: 'aprev k-' + a.kind });
    var file = a.file || (/^[a-zA-Z]:[\\/]|^\//.test(a.path || '') ? a.path : null);
    if (a.kind === 'sfx' || a.kind === 'music') {
      prev.appendChild(h('div', { class: 'awave' }, a.kind === 'music' ? '♫' : '◉'));
      if (file && a.exists !== false) prev.appendChild(h('audio', { controls: true, preload: 'metadata', src: R.fileUrl(file), class: 'aplayer' }));
      else prev.appendChild(h('div', { class: 'muted small' }, 'file missing'));
    } else if (a.kind === 'image' && file) {
      prev.appendChild(h('img', { src: R.fileUrl(file), alt: a.name, loading: 'lazy' }));
    } else if (a.kind === 'font' || a.kind === 'caption_style') {
      prev.appendChild(h('span', { class: 'muted small' }, 'rendering preview...'));
      lazyShow(prev, a);
    } else if (a.kind === 'layout') {
      prev.appendChild(layoutDiagram(a.name));
    }
    var tags = (a.tags || []).filter(function (t) { return t !== 'built-in'; }).slice(0, 6);
    var acts = h('div', { class: 'row acts' });
    useButtons(a).forEach(function (b) { acts.appendChild(b); });
    if (a.scope !== 'built-in' && R.has('asset.remove')) {
      acts.appendChild(h('span', { class: 'grow' }));
      acts.appendChild(R.button('Remove', function () {
        return R.confirm('Remove asset', 'Remove "' + (a.name || a.id) + '" from the ' + a.scope + ' library? Files copied into the library are deleted.', 'Remove').then(function (ok) {
          if (!ok) return;
          return R.run('asset.remove', { id: a.id, scope: a.scope === 'global' ? 'global' : 'project' }).then(function (d) { if (d) { R.toast('Removed ' + d.removed, 'good'); return reload(); } });
        });
      }, { small: true, cls: 'ghost danger' }));
    }
    return h('div', { class: 'card acard', 'data-id': a.id, 'data-kind': a.kind },
      prev,
      h('div', { class: 'ainfo' },
        h('div', { class: 'aname', title: a.name }, a.name || a.id),
        h('div', { class: 'row tight' }, R.badge(KIND_LABEL[a.kind] || a.kind, 'kind'), R.badge(a.scope, a.scope === 'built-in' ? 'none' : a.scope === 'global' ? 'info' : 'pass'),
          h('span', { class: 'small lic', title: (a.attribution || '') + (a.license_url ? '\n' + a.license_url : '') }, a.license || 'no license')),
        tags.length ? h('div', { class: 'atags' }, tags.map(function (t) { return h('span', { class: 'tag' }, t); })) : null,
        h('div', { class: 'mono tiny muted', title: 'use as asset:' + a.id }, a.id)),
      acts);
  }
  function lazyShow(box, a) {
    var go = function () {
      showAsset(a.id).then(function (d) {
        R.clear(box);
        var pv = d && d.preview;
        if (pv && pv.image) {
          var img = h('img', { src: R.fileUrl(pv.image) + '&v=' + d._v, alt: a.name + ' preview', class: a.kind === 'caption_style' ? 'band' : 'specimen' });
          box.appendChild(img);
          if (a.kind === 'caption_style') box.title = 'libass render of the style (exact)';
        } else box.appendChild(h('span', { class: 'muted small' }, (pv && pv.note) || 'no preview'));
      });
    };
    if (window.IntersectionObserver) {
      var io = new IntersectionObserver(function (ents) { if (ents.some(function (e) { return e.isIntersecting; })) { io.disconnect(); go(); } });
      setTimeout(function () { io.observe(box); }, 0);
    } else go();
  }
  // Built-in layout schematic (9:16 canvas with the panes).
  function layoutDiagram(name) {
    var d = h('div', { class: 'ldiag' });
    if (name === 'split-vertical') { d.appendChild(h('i', { style: { left: '0', top: '0', width: '100%', height: '50%' } }, 'top')); d.appendChild(h('i', { style: { left: '0', top: '50%', width: '100%', height: '50%' } }, 'bottom')); }
    else if (name === 'blur-fit') { d.classList.add('blur'); d.appendChild(h('i', { style: { left: '0', top: '34%', width: '100%', height: '32%' } }, 'whole frame')); }
    else d.appendChild(h('i', { style: { left: '0', top: '0', width: '100%', height: '100%' } }, 'crop'));
    return d;
  }
  function layoutPresetCard(p) {
    var acts = h('div', { class: 'row acts' });
    if (R.has('layout.apply_preset')) acts.appendChild(useBtn('Apply to clip', function (clip) { return R.run('layout.apply_preset', { clip_id: clip, name: p.name }); }, 'layout ' + p.name + ' applied'));
    return h('div', { class: 'card acard', 'data-id': 'layout:' + p.name, 'data-kind': 'layout' },
      h('div', { class: 'aprev k-layout' }, h('div', { class: 'ldiag custom' }, h('i', { style: { left: '0', top: '0', width: '100%', height: '100%' } }, (p.regions ? p.regions.length + ' regions' : 'custom')))),
      h('div', { class: 'ainfo' }, h('div', { class: 'aname' }, p.name), h('div', { class: 'row tight' }, R.badge('layout', 'kind'), R.badge('project preset', 'pass')),
        h('div', { class: 'small muted' }, p.description || 'saved with layout.save_preset')),
      acts);
  }

  // "Use in clip" actions; each confirms the change with the matching read command.
  function useBtn(label, fn, okMsg, check) {
    return R.button(label, function () {
      var clip = st.clipSel && st.clipSel.value;
      if (!clip) { R.toast('Create a clip first (Clips page)', 'bad'); return; }
      return fn(clip).then(function (d) {
        if (!d) return;
        var t = R.toast(okMsg + ' → ' + (st.clipSel.selectedOptions[0] ? st.clipSel.selectedOptions[0].textContent : clip), 'good');
        if (check) check(clip, d);
        return d;
      });
    }, { small: true, primary: true });
  }
  function useButtons(a) {
    var out = [];
    if (a.kind === 'sfx' && R.has('audio.add_sfx')) {
      out.push(useBtn('+ Add at time', function (clip) {
        var t = Number(st.atEl.value) || 0;
        return R.run('audio.add_sfx', { clip_id: clip, file: 'asset:' + a.id, at: R.round(t, 3) });
      }, 'SFX added', function (clip, d) { R.toast('sfx ' + d.sfx_id + ' plays at ' + R.fmt(d.sfx && d.sfx.resolved_at)); }));
    }
    if (a.kind === 'music' && R.has('audio.add_music')) {
      out.push(useBtn('Set as music bed', function (clip) { return R.run('audio.add_music', { clip_id: clip, path: 'asset:' + a.id }); }, 'Music bed set'));
    }
    if (a.kind === 'caption_style') {
      var cmd = R.has('caption.set') ? 'caption.set' : (R.has('clip.set_captions') ? 'clip.set_captions' : null);
      if (cmd) out.push(useBtn('Use on clip', function (clip) { var i = { clip_id: clip, enabled: true }; i[cmd === 'caption.set' ? 'style' : 'preset'] = a.name; return R.run(cmd, i); }, 'Caption style ' + a.name));
      if (R.has('caption.create_style')) out.push(R.button('Duplicate…', function () { openCreator({ base: a.name }); }, { small: true }));
    }
    if (a.kind === 'layout' && R.has('layout.apply_preset')) {
      var needs = a.name === 'split-vertical';
      var b = useBtn('Apply to clip', function (clip) { return R.run('layout.apply_preset', { clip_id: clip, name: a.name }); }, 'Layout ' + a.name);
      if (needs) { b.disabled = true; b.title = 'split-vertical needs top/bottom boxes: use the editor Layout tab (Streamer split)'; }
      out.push(b);
    }
    if (a.kind === 'font' && R.has('caption.create_style')) {
      out.push(R.button('Caption style with this font…', function () { openCreator({ font_family: fontFamily(a) }); }, { small: true }));
    }
    return out;
  }
  // "Anton Regular" -> "Anton"; "Poppins ExtraBold" stays (libass matches the full name).
  function fontFamily(a) {
    var n = String(a.name || '').replace(/\s+(Regular|Book|Normal)$/i, '');
    return n;
  }

  // ------------------------------------------------------------ add asset
  function addForm() {
    var f = {
      path: h('input', { type: 'text', placeholder: 'C:\\sounds\\whoosh.wav (copied into the library)', class: 'grow' }),
      kind: R.select([['sfx', 'sfx'], ['music', 'music'], ['font', 'font'], ['image', 'image'], ['caption_style', 'caption style (json)'], ['layout', 'layout (json)']], 'sfx'),
      license: h('input', { type: 'text', list: 'lic-list', placeholder: 'required, e.g. CC0-1.0' }),
      name: h('input', { type: 'text', placeholder: 'default: file name' }),
      tags: h('input', { type: 'text', placeholder: 'whoosh, transition' }),
      scope: R.select([['project', 'this project'], ['global', 'global (all projects)']], 'project'),
      attribution: h('input', { type: 'text', placeholder: 'credit line if the license needs it' }),
      source_url: h('input', { type: 'text', placeholder: 'https://… where it came from' }),
      license_url: h('input', { type: 'text', placeholder: 'https://… license text' })
    };
    var dl = h('datalist', { id: 'lic-list' }, LICENSES.map(function (l) { return h('option', { value: l }); }));
    var err = h('div', { class: 'err small', hidden: true });
    var box = h('div', { class: 'card addasset' }, h('h2', null, 'Add to the library'), dl,
      h('div', { class: 'row' }, R.field('File path *', f.path)),
      h('div', { class: 'fgrid3' }, R.field('Kind *', f.kind), R.field('License *', f.license, 'unlicensed media is refused'), R.field('Scope', f.scope),
        R.field('Name', f.name), R.field('Tags', f.tags, 'comma separated'), R.field('Attribution', f.attribution), R.field('Source URL', f.source_url), R.field('License URL', f.license_url)),
      err,
      h('div', { class: 'row' }, R.button('Add asset', function () {
        err.hidden = true;
        var path = f.path.value.trim().replace(/^"|"$/g, ''), lic = f.license.value.trim();
        if (!path) { err.textContent = 'Enter the path of a local file.'; err.hidden = false; return; }
        if (!lic || lic === 'licensed:') { err.textContent = 'A license is required (e.g. CC0-1.0, CC-BY-4.0, OFL-1.1, own work).'; err.hidden = false; f.license.focus(); return; }
        var input = { path: path, kind: f.kind.value, license: lic, scope: f.scope.value };
        if (f.name.value.trim()) input.name = f.name.value.trim();
        var tags = f.tags.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
        if (tags.length) input.tags = tags;
        ['attribution', 'source_url', 'license_url'].forEach(function (k) { if (f[k].value.trim()) input[k] = f[k].value.trim(); });
        return R.run('asset.add', input).then(function (d) {
          if (!d) return;
          var a = d.asset || {};
          R.toast('Added ' + (a.id || 'asset') + ' (' + (a.scope || input.scope) + ')', 'good');
          (d.warnings || []).forEach(function (w) { R.toast(w); });
          f.path.value = ''; f.name.value = ''; f.tags.value = '';
          st.kind = a.kind || st.kind; R.store('rustler.assets.kind', st.kind);
          if (st.reload) st.reload();
        });
      }, { primary: true, need: ['asset.add'] }), h('span', { class: 'muted small' }, 'Same as `rustler asset add <path> --kind … --license …`')));
    return box;
  }

  // ------------------------------------------------------------ caption style creator
  // R.styleCreator(opts) -> element. opts: {base, font_family, onSaved(styleName, data), applyClip (editor: E)}.
  R.styleCreator = function (opts) {
    var o = opts || {};
    var E = o.editor || null;
    var f = {
      name: h('input', { type: 'text', placeholder: 'e.g. neon-pop', value: o.name || '' }),
      base: h('select', null, R.option(o.base || 'bold-karaoke')),
      font_family: h('input', { type: 'text', list: 'font-fams', value: o.font_family || '' }),
      size_factor: h('input', { type: 'range', min: 0.02, max: 0.15, step: 0.001 }),
      uppercase: h('input', { type: 'checkbox' }),
      primary: h('input', { type: 'color', value: '#ffffff' }),
      outline: h('input', { type: 'color', value: '#000000' }),
      outline_width: h('input', { type: 'number', min: 0, max: 20, step: 0.5, class: 'num' }),
      shadow: h('input', { type: 'number', min: 0, max: 20, step: 0.5, class: 'num' }),
      active_kind: R.select([['recolor', 'recolor (spoken word changes colour)'], ['box', 'box (word on a box)'], ['pop', 'pop (colour + scale pop)']], 'recolor'),
      active_color: h('input', { type: 'color', value: '#ffe000' }),
      scope: R.select([['project', 'this project'], ['global', 'global']], 'project'),
      description: h('input', { type: 'text', placeholder: 'optional' }),
      tags: h('input', { type: 'text', placeholder: 'optional, comma separated' })
    };
    var sizeV = h('span', { class: 'mono small' }, '');
    var fams = h('datalist', { id: 'font-fams' });
    var canvas = h('canvas', { class: 'stylecanvas', width: 360, height: 220 });
    var exact = h('div', { class: 'styleexact' }, h('span', { class: 'muted small' }, 'Save to see the exact libass render.'));
    var baseStyle = null;
    function load(baseName, keepFont) {
      return showAsset('style-' + baseName, true).then(function (d) {
        var s = d && d.style;
        if (!s) return;
        baseStyle = s;
        if (!keepFont || !f.font_family.value) f.font_family.value = s.font_family;
        f.size_factor.value = s.size_factor; f.uppercase.checked = !!s.uppercase;
        f.primary.value = hex(s.primary, '#ffffff'); f.outline.value = hex(s.outline_colour, '#000000');
        f.outline_width.value = s.outline; f.shadow.value = s.shadow;
        f.active_kind.value = s.active_kind || 'recolor'; f.active_color.value = hex(s.active_colour, '#ffe000');
        draw();
        if (d.preview && d.preview.image) showExact(d.preview.image, d._v, 'base ' + baseName + ' (libass)');
      });
    }
    function hex(c, dflt) { var m = /^#?([0-9a-fA-F]{6})/.exec(String(c || '')); return m ? '#' + m[1].toLowerCase() : dflt; }
    function showExact(path, v, label) {
      R.clear(exact).appendChild(h('div', { class: 'muted small' }, label));
      exact.appendChild(h('div', { class: 'bandbox' }, h('img', { src: R.fileUrl(path) + '&v=' + (v || Date.now()), alt: 'exact style preview', class: 'band' })));
    }
    // Live approximation on a canvas (fonts: Anton / Poppins ExtraBold are loaded by the page).
    function draw() {
      var c = canvas.getContext('2d'), W = canvas.width, H = canvas.height;
      var g = c.createLinearGradient(0, 0, 0, H); g.addColorStop(0, '#5b6270'); g.addColorStop(1, '#2e333c');
      c.fillStyle = g; c.fillRect(0, 0, W, H);
      sizeV.textContent = Number(f.size_factor.value).toFixed(3);
      var fam = f.font_family.value || 'Anton';
      var css = /anton/i.test(fam) ? R.fonts.anton : /poppins/i.test(fam) ? R.fonts.poppins : '"' + fam + '", sans-serif';
      var k = W / 1080 * 2.2;   // zoomed: the caption band of a 1080-wide frame
      var size = 1080 * Number(f.size_factor.value) * k / (/anton/i.test(fam) ? 1.7 : 1.4);
      c.font = (/anton/i.test(fam) ? '400 ' : '800 ') + Math.round(size) + 'px ' + css;
      c.textBaseline = 'alphabetic';
      var words = ['Ask', 'not', 'what'].map(function (w) { return f.uppercase.checked ? w.toUpperCase() : w; });
      var sp = c.measureText(' ').width, tw = words.reduce(function (s, w) { return s + c.measureText(w).width; }, 0) + sp * 2;
      var x = W / 2 - tw / 2, y = H * 0.62;
      var kind = f.active_kind.value;
      words.forEach(function (w, i) {
        var ww = c.measureText(w).width, on = i === 1;
        if (on && kind === 'box') { var pad = 12 * k; c.fillStyle = f.active_color.value; c.fillRect(x - pad, y - size * 0.95, ww + 2 * pad, size * 1.25); }
        var sh = Number(f.shadow.value) || 0;
        if (sh > 0) { c.fillStyle = 'rgba(0,0,0,.6)'; c.fillText(w, x + sh * k, y + sh * k); }
        var ow = Number(f.outline_width.value) || 0;
        if (ow > 0) { c.lineWidth = ow * k * 2; c.strokeStyle = f.outline.value; c.lineJoin = 'round'; c.strokeText(w, x, y); }
        c.fillStyle = on && kind !== 'box' ? f.active_color.value : f.primary.value;
        if (on && kind === 'pop') { c.save(); c.translate(x + ww / 2, y); c.scale(1.1, 1.1); c.fillText(w, -ww / 2, 0); c.restore(); } else c.fillText(w, x, y);
        x += ww + sp;
      });
      c.font = '11px "Segoe UI", sans-serif'; c.fillStyle = 'rgba(255,255,255,.55)'; c.fillText('live approximation · spoken word = "' + words[1] + '"', 8, H - 8);
    }
    Object.keys(f).forEach(function (k) { f[k].addEventListener('input', draw); f[k].addEventListener('change', draw); });
    f.base.addEventListener('change', function () { load(f.base.value, false); });
    // style + font lists
    R.loadStyleDefs().then(function (list) {
      R.clear(f.base);
      list.forEach(function (s) { f.base.appendChild(R.option(s.name, s.name + (s.built_in ? '' : ' (custom)'), s.name === (o.base || 'bold-karaoke'))); });
      var famSet = {};
      list.forEach(function (s) { if (s.font_family) famSet[s.font_family] = 1; });
      return R.has('asset.list') ? R.run('asset.list', { kind: 'font' }, true).then(function (d) {
        R.listOf(d, 'assets').forEach(function (a) { famSet[fontFamily(a)] = 1; });
        Object.keys(famSet).sort().forEach(function (n) { fams.appendChild(h('option', { value: n })); });
      }) : null;
    }).then(function () { return load(f.base.value || o.base || 'bold-karaoke', !!o.font_family); });

    function input() {
      var name = f.name.value.trim();
      if (!name) throw new Error('Give the style a name');
      var i = { name: name, base: f.base.value, font_family: f.font_family.value.trim() || undefined, size_factor: R.round(Number(f.size_factor.value), 4),
        uppercase: f.uppercase.checked, primary: f.primary.value.toUpperCase(), outline: f.outline.value.toUpperCase(),
        outline_width: Number(f.outline_width.value), shadow: Number(f.shadow.value), active_kind: f.active_kind.value, scope: f.scope.value };
      if (i.active_kind === 'box') i.box_color = f.active_color.value.toUpperCase(); else i.active_color = f.active_color.value.toUpperCase();
      if (f.description.value.trim()) i.description = f.description.value.trim();
      var tags = f.tags.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
      if (tags.length) i.tags = tags;
      return i;
    }
    function save() {
      var i;
      try { i = input(); } catch (e) { R.toast(e.message, 'bad'); return Promise.resolve(null); }
      return R.run('caption.create_style', i).then(function (d) {
        if (!d) return null;
        (d.warnings || []).forEach(function (w) { R.toast(w); });
        R.toast('Style saved: ' + d.style.name + ' (' + d.scope + ')', 'good');
        R.styleDefs[d.style.name] = { name: d.style.name, font_family: d.style.font_family, primary: d.style.primary, active_colour: d.style.active_colour, active_kind: d.style.active_kind, uppercase: d.style.uppercase,
          size_factor: d.style.size_factor, outline: d.style.outline, outline_colour: d.style.outline_colour, base: d.style.base };
        return d;
      });
    }
    var btns = h('div', { class: 'row' },
      R.button('Save + exact preview', function () {
        return save().then(function (d) {
          if (!d) return;
          return showAsset(d.asset_id, true).then(function (s) { if (s && s.preview && s.preview.image) showExact(s.preview.image, s._v, d.style.name + ' · libass render (exact)'); if (o.onSaved) o.onSaved(d.style.name, d); });
        });
      }, { primary: !E, small: true, need: ['caption.create_style'] }),
      E ? R.button('Save + apply to this clip', function () {
        return save().then(function (d) {
          if (!d) return;
          var cmd = R.has('caption.set') ? 'caption.set' : 'clip.set_captions';
          var inp = { enabled: true }; inp[cmd === 'caption.set' ? 'style' : 'preset'] = d.style.name;
          return E.mutate(cmd, inp, { toast: 'Clip now uses ' + d.style.name }).then(function (r) {
            if (!r) return;
            return R.run('render.preview_frame', { clip_id: E.clipId, out_t: R.round(E.t, 3) }).then(function (p) {
              if (p) showExact(p.image, Date.now(), 'Exact frame of this clip at ' + R.tc(E.t, E.fps) + ' · ' + d.style.name);
              if (o.onSaved) o.onSaved(d.style.name, d);
            });
          });
        });
      }, { primary: true, small: true, need: ['caption.create_style', 'render.preview_frame'] }) : null);
    var form = h('div', { class: 'stylecreator' }, fams,
      h('div', { class: 'scgrid' },
        h('div', { class: 'scform' },
          h('div', { class: 'fgrid2' }, R.field('Name *', f.name), R.field('Start from', f.base), R.field('Font family', f.font_family, 'as libass sees it'), R.field('Scope', f.scope)),
          R.field('Size (fraction of width)', h('div', { class: 'row tight' }, f.size_factor, sizeV)),
          h('div', { class: 'fgrid3' }, R.field('Text', f.primary), R.field('Outline', f.outline), R.field('Outline px', f.outline_width), R.field('Shadow px', f.shadow),
            R.field('Spoken word', f.active_kind), R.field('Word colour / box', f.active_color)),
          h('label', { class: 'chk' }, f.uppercase, ' UPPERCASE'),
          h('div', { class: 'fgrid2' }, R.field('Description', f.description), R.field('Tags', f.tags))),
        h('div', { class: 'scprev' }, canvas, exact)),
      btns);
    return form;
  };
  function openCreator(opts) {
    var el = R.styleCreator(Object.assign({}, opts || {}, { onSaved: function () { if (st.reload) st.reload(); } }));
    R.modal('New caption style (caption.create_style)', el);
  }
  R.openStyleCreator = openCreator;

  R.views.assets = { label: 'Assets', icon: '◈', render: render };
})();
