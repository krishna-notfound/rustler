// Clips page: create a clip and open clips in the editor.
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  function render(v) {
    v.appendChild(h('h1', null, 'Clips'));
    var miss = R.missing(['clip.list']);
    if (miss.length) { v.appendChild(R.na.apply(null, miss)); return; }
    var form = h('div', { class: 'card' });
    var list = h('div', { class: 'clipgrid' });
    v.appendChild(form);
    v.appendChild(h('h2', null, 'All clips'));
    v.appendChild(list);

    Promise.all([R.loadSources(), R.loadStyles(), R.has('layout.presets') ? R.run('layout.presets', {}, true) : null]).then(function (res) {
      var sources = res[0], styles = res[1], presets = res[2];
      var layouts = presets ? (presets.built_in || []).map(function (p) { return p.name; }).filter(function (n) { return n !== 'split-vertical'; }) : ['center-crop', 'blur-fit'];
      var name = h('input', { placeholder: 'clip name', value: 'Clip ' + (Date.now() % 1000) });
      var src = R.select(sources.map(function (s) { return [R.sid(s), s.name || R.sid(s)]; }), S.sel.source_id);
      var su = h('input', { type: 'number', min: 0, class: 'num', placeholder: 'start #', value: S.sel.start_unit });
      var eu = h('input', { type: 'number', min: 0, class: 'num', placeholder: 'end #', value: S.sel.end_unit });
      var ss = h('input', { type: 'number', step: '0.1', class: 'num', placeholder: 'start s' });
      var es = h('input', { type: 'number', step: '0.1', class: 'num', placeholder: 'end s' });
      var lay = R.select(layouts, 'center-crop');
      var sty = R.select([['', 'default captions']].concat(styles.map(function (x) { return [x.name, x.name]; })), '');
      form.appendChild(h('h2', null, 'Create clip'));
      form.appendChild(h('div', { class: 'row' }, R.field('Name', name), R.field('Source', src)));
      form.appendChild(h('div', { class: 'row' }, R.field('Start unit', su), R.field('End unit', eu), h('span', { class: 'muted' }, 'or'), R.field('Start s', ss), R.field('End s', es),
        R.field('Layout', lay), R.field('Captions', sty),
        R.button('Create + edit', function () {
          var b = { name: name.value.trim() || 'clip', source_id: src.value, layout: lay.value, captions: true };
          if (!b.source_id) { R.toast('Import a source first', 'bad'); return; }
          if (su.value !== '') b.start_unit = Number(su.value);
          if (eu.value !== '') b.end_unit = Number(eu.value);
          if (ss.value !== '') b.start = Number(ss.value);
          if (es.value !== '') b.end = Number(es.value);
          if (sty.value) b.caption_style = sty.value;
          return R.createClip(b);
        }, { primary: true, need: ['clip.create'] })));
      if (!sources.length) form.appendChild(h('div', { class: 'muted small' }, 'No sources yet, import one on the Sources page.'));
      form.appendChild(h('div', { class: 'muted small' }, 'Tip: pick units on Sources > Transcript (start / end), or use Highlights there.'));
    });

    R.run('clip.list', {}).then(function (d) {
      var items = R.listOf(d, 'clips');
      if (!items.length) { list.appendChild(h('p', { class: 'muted' }, 'No clips yet.')); return; }
      items.forEach(function (c) { list.appendChild(clipCard(c)); });
    });
  }
  function clipCard(c) {
    var id = R.cid(c);
    var thumb = h('div', { class: 'cthumb' });
    var card = h('div', { class: 'card clipcard' }, thumb,
      h('div', { class: 'cinfo' },
        h('h2', null, c.name || id),
        h('div', { class: 'muted small mono' }, id),
        h('div', { class: 'small' }, c.summary || ((c.segments || 0) + ' segments')),
        h('div', { class: 'row' },
          R.button('Open editor', function () { R.store('rustler.lastClip', id); R.go('editor', id); }, { primary: true }),
          R.button('Renders', function () { S.renderClip = id; R.go('renders'); }, { need: ['clip.show'] }))));
    // Thumbnail: the latest render's first frame, else nothing (preview_frame is on demand in the editor).
    return card;
  }
  R.views.clips = { label: 'Clips', icon: '✂', render: render };
})();
