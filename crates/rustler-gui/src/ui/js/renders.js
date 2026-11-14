// Renders page + shared QC report view.
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  R.renderQc = function (box, rep) {
    var r = rep.report || rep;
    var checks = R.listOf(r, 'checks');
    box.appendChild(h('h3', null, 'QC ' + (r.passed === true ? '- passed' : r.passed === false ? '- failed' : '')));
    if (checks.length) {
      box.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl' },
        h('thead', null, h('tr', null, h('th', null, 'check'), h('th', null, 'status'), h('th', null, 'value'), h('th', null, 'expected'), h('th', null, 'hint'))),
        h('tbody', null, checks.map(function (c) {
          var st = String(c.status || '').toLowerCase();
          return h('tr', null, h('td', null, c.name), h('td', { class: 'st-' + st }, st),
            h('td', { class: 'mono' }, R.short(c.value, 60)), h('td', null, c.expected || ''), h('td', { class: 'muted' }, c.fix_hint || ''));
        })))));
    } else box.appendChild(h('pre', null, R.pretty(rep)));
    if (r.contact_sheet) { box.appendChild(h('h3', null, 'Contact sheet')); box.appendChild(R.img(r.contact_sheet)); }
  };

  function render(v) {
    v.appendChild(h('h1', null, 'Renders'));
    var miss = R.missing(['clip.list', 'clip.show']);
    if (miss.length) { v.appendChild(R.na.apply(null, miss)); return; }
    var sel = h('select');
    var rsel = h('select');
    var stage = h('div');
    v.appendChild(h('div', { class: 'row' }, h('span', { class: 'muted' }, 'Clip'), sel, h('span', { class: 'muted' }, 'Render'), rsel,
      R.button('Open in editor', function () { if (sel.value) R.go('editor', sel.value); })));
    v.appendChild(stage);
    var current = null;

    function showRender(r) {
      R.clear(stage);
      if (!r) { stage.appendChild(h('p', { class: 'muted' }, 'No renders for this clip yet. Render it from the editor (Render tab).')); return; }
      stage.appendChild(h('div', { class: 'card renderview' },
        h('video', { controls: true, preload: 'metadata', src: R.fileUrl(r.path) }),
        h('div', null,
          h('div', { class: 'muted small mono' }, String(r.path)),
          h('div', { class: 'small' }, (r.encoder || '') + '  |  ' + (r.quality || '') + '  |  ' + String(r.created_at || '').replace('T', ' ').slice(0, 19)),
          h('div', { class: 'row' },
            R.button('Export', function () {
              return R.run('export.clip', { clip_id: R.cid(current), render_id: r.id }).then(function (d) { if (d) { R.toast('Exported', 'good'); qcBox.appendChild(R.resultView(d)); } });
            }, { need: ['export.clip'] })))));
      var qcBox = h('div', { class: 'card' });
      stage.appendChild(qcBox);
      function drawQc(rep) { R.clear(qcBox); if (rep) R.renderQc(qcBox, rep); }
      qcBox.appendChild(h('div', { class: 'row' }, R.button('Run QC', function () {
        return R.run('qc.run', { clip_id: R.cid(current), render_id: r.id, allow_fail: true }).then(drawQc);
      }, { primary: true, need: ['qc.run'] }), h('span', { class: 'muted small' }, r.qc ? 'loading saved QC...' : 'no QC yet for this render')));
      if (r.qc) R.getJson(R.optUrl(r.qc)).then(function (rep) { if (rep) drawQc(rep); });
    }
    function pickClip(id) {
      return R.run('clip.show', { clip_id: id }).then(function (d) {
        if (!d) return;
        var c = d.clip && typeof d.clip === 'object' ? d.clip : d;
        current = c;
        if (!c.id && !c.clip_id) c.id = id;
        var rs = Array.isArray(c.renders) ? c.renders : [];
        R.clear(rsel);
        rs.forEach(function (r, i) { rsel.appendChild(R.option(String(i), (r.id || 'render') + '  ' + String(r.created_at || '').slice(0, 19), i === rs.length - 1)); });
        rsel.onchange = function () { showRender(rs[Number(rsel.value)]); };
        showRender(rs[rs.length - 1]);
      });
    }
    R.run('clip.list', {}).then(function (d) {
      var items = R.listOf(d, 'clips');
      if (!items.length) { stage.appendChild(h('p', { class: 'muted' }, 'No clips yet.')); return; }
      items.forEach(function (c) { sel.appendChild(R.option(R.cid(c), c.name || R.cid(c), R.cid(c) === S.renderClip)); });
      sel.onchange = function () { pickClip(sel.value); };
      pickClip(sel.value);
    });
  }
  R.views.renders = { label: 'Renders', icon: '◎', render: render };
})();
