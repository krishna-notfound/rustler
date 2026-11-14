// Project page: open / create a project folder.
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  function render(v) {
    v.appendChild(h('h1', null, 'Project'));
    var path = h('input', { class: 'grow', placeholder: 'path to project folder, e.g. C:\\work\\my-project', value: S.project });
    var info = h('div');
    function showInfo() {
      R.clear(info);
      var d = S.projectInfo;
      if (!d) { info.appendChild(h('p', { class: 'muted' }, 'No project open yet. Enter a folder and press Open (existing) or Create (new).')); return; }
      var raw = d._raw || d;
      var p = raw.project || raw;
      var counts = raw.counts || {};
      var nSrc = counts.sources !== undefined ? counts.sources : (Array.isArray(p.sources) ? p.sources.length : '?');
      var nClip = counts.clips !== undefined ? counts.clips : (Array.isArray(p.clips) ? p.clips.length : '?');
      info.appendChild(h('div', { class: 'card' },
        h('h2', null, p.name || 'project'),
        h('div', { class: 'muted small mono' }, p.id || ''),
        h('div', { class: 'stats' },
          h('div', { class: 'stat' }, h('b', null, nSrc), h('span', null, 'sources')),
          h('div', { class: 'stat' }, h('b', null, nClip), h('span', null, 'clips')),
          h('div', { class: 'stat' }, h('b', null, p.revision === undefined ? '?' : p.revision), h('span', null, 'revision'))),
        h('div', { class: 'row' },
          R.button('Sources', function () { R.go('sources'); }), R.button('Clips', function () { R.go('clips'); })),
        h('details', { class: 'raw' }, h('summary', null, 'raw project.show'), h('pre', null, R.pretty(raw)))));
    }
    function open() {
      S.project = path.value.trim();
      R.store('rustler.project', S.project);
      return R.refreshProject(false).then(function (d) { if (d) R.toast('Project opened', 'good'); showInfo(); });
    }
    function create() {
      var dir = path.value.trim();
      if (!dir) { R.toast('Enter a folder path first', 'bad'); return; }
      var name = dir.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || 'project';
      S.project = dir;
      R.store('rustler.project', S.project);
      return R.run('project.create', { name: name, dir: dir }).then(function (d) {
        if (d) { R.toast('Project created', 'good'); return R.refreshProject(true).then(showInfo); }
      });
    }
    var miss = R.missing(['project.show']);
    v.appendChild(h('div', { class: 'card' },
      h('div', { class: 'row' }, path,
        R.button('Open', open, { primary: true, need: ['project.show'] }),
        R.button('Create', create, { need: ['project.create'] })),
      h('div', { class: 'muted small' }, 'Leave empty to use the project the server was started with.')));
    if (miss.length) v.appendChild(R.na.apply(null, miss));
    v.appendChild(info);
    showInfo();
  }
  R.views.project = { label: 'Project', icon: '\u25A3', render: render };
})();
