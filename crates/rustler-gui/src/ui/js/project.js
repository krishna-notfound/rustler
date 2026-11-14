// Project page: open / create a project (project.open|create), recent projects (project.list),
// the status dashboard (project.status: sources analyzed?, clips with render/QC/brief/export
// badges, suggestions with Run buttons) and pack / unpack (project.pack|unpack).
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  function openDir(dir) {
    S.project = dir;
    R.store('rustler.project', S.project);
    var p = R.has('project.open') && dir ? R.run('project.open', { dir: dir }) : Promise.resolve(true);
    return p.then(function (ok) {
      if (!ok) return null;
      return R.refreshProject(false).then(function (d) { if (d) { R.toast('Project opened', 'good'); R.go('project'); } return d; });
    });
  }
  R.openProject = openDir;

  function render(v) {
    v.appendChild(h('div', { class: 'pagehead' }, h('h1', null, 'Project'), h('span', { class: 'muted' }, S.project || (S.projectInfo ? 'server default project' : ''))));
    var path = h('input', { class: 'grow', placeholder: 'path to project folder, e.g. C:\\work\\my-project', value: S.project });
    function create() {
      var dir = path.value.trim();
      if (!dir) { R.toast('Enter a folder path first', 'bad'); return; }
      var name = dir.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || 'project';
      S.project = dir;
      R.store('rustler.project', S.project);
      return R.run('project.create', { name: name, dir: dir }).then(function (d) {
        if (d) { R.toast('Project created', 'good'); return R.refreshProject(true).then(function () { R.go('project'); }); }
      });
    }
    v.appendChild(h('div', { class: 'card' },
      h('div', { class: 'row' }, path,
        R.button('Open', function () { return openDir(path.value.trim()); }, { primary: true, need: ['project.show'] }),
        R.button('Create', create, { need: ['project.create'] })),
      h('div', { class: 'muted small' }, 'Leave empty to use the project the server was started with.')));
    var miss = R.missing(['project.show']);
    if (miss.length) v.appendChild(R.na.apply(null, miss));

    var grid = h('div', { class: 'projgrid' });
    var dash = h('div');
    v.appendChild(dash);
    v.appendChild(grid);
    if (S.projectInfo && R.has('project.status')) statusDash(dash);
    else if (S.projectInfo) dash.appendChild(basicInfo());
    else dash.appendChild(h('p', { class: 'muted' }, 'No project open yet. Pick a recent one below, or enter a folder and press Open (existing) or Create (new).'));
    if (R.has('project.list')) grid.appendChild(recent());
    if (R.has('project.pack') || R.has('project.unpack')) grid.appendChild(packCard());
  }

  function basicInfo() {
    var d = S.projectInfo, raw = d._raw || d, p = raw.project || raw;
    return h('div', { class: 'card' }, h('h2', null, p.name || 'project'), h('div', { class: 'muted small mono' }, p.id || ''),
      h('details', { class: 'raw' }, h('summary', null, 'raw project.show'), h('pre', null, R.pretty(raw))));
  }

  // ------------------------------------------------------------ status dashboard
  function statusDash(box) {
    box.appendChild(h('div', { class: 'card muted' }, 'loading project.status...'));
    return R.run('project.status', {}, true).then(function (d) {
      R.clear(box);
      if (!d) { box.appendChild(basicInfo()); return; }
      var srcs = d.sources || [], clips = d.clips || [];
      var analyzed = srcs.filter(function (s) { return s.analyzed; }).length;
      var rendered = clips.filter(function (c) { return c.renders > 0; }).length;
      var qcPass = clips.filter(function (c) { return c.qc && /pass/i.test(qcStatus(c.qc)); }).length;
      var exported = clips.filter(function (c) { return c.exported; }).length;
      var card = h('div', { class: 'card statusdash' },
        h('div', { class: 'row' }, h('h2', null, d.name || 'project'), h('span', { class: 'muted small mono' }, d.project_dir || ''), h('span', { class: 'grow' }),
          R.button('Refresh', function () { return statusDash(R.clear(box)); }, { small: true, cls: 'ghost' })),
        h('div', { class: 'stats' },
          stat(srcs.length, 'sources'), stat(analyzed + '/' + srcs.length, 'analyzed'), stat(clips.length, 'clips'),
          stat(rendered + '/' + clips.length, 'rendered'), stat(qcPass + '/' + clips.length, 'QC pass'), stat(exported + '/' + clips.length, 'exported'),
          stat(d.revision === undefined ? '?' : d.revision, 'revision')));
      box.appendChild(card);

      // suggestions
      var sug = d.suggestions || [];
      if (sug.length) {
        var sb = h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, 'Next steps (project.status suggestions)')));
        sug.forEach(function (line) { sb.appendChild(R.cliRow(line, { done: function () { statusDash(R.clear(box)); } })); });
        card.appendChild(sb);
      } else card.appendChild(h('div', { class: 'small st-pass' }, 'Nothing unfinished.'));

      // sources
      var ss = h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, 'Sources'), h('a', { class: 'small', onclick: function () { R.go('sources'); } }, 'open Sources')));
      if (!srcs.length) ss.appendChild(h('div', { class: 'muted small' }, 'No sources yet.'));
      srcs.forEach(function (s) {
        ss.appendChild(h('div', { class: 'row srcline', 'data-source': s.id },
          h('b', { class: 'ell', style: { maxWidth: '280px' } }, s.name || s.id), h('span', { class: 'muted small mono' }, R.fmt(s.duration)),
          h('div', { class: 'srcstat' },
            R.badge(s.analyzed ? 'transcript ' + s.transcript_words + ' words' : 'not analyzed', s.analyzed ? 'pass' : 'warn'),
            s.units !== undefined ? R.badge(s.units + ' units', s.units ? 'pass' : 'none') : null,
            R.badge('scenes', s.scenes ? 'pass' : 'none'), R.badge('audio', s.audio_events ? 'pass' : 'none'),
            R.badge('deep', s.deep_analysis ? 'pass' : 'none', (s.deep_parts || []).join(', ')),
            s.speakers !== undefined ? R.badge('speakers', s.speakers ? 'pass' : 'none') : null,
            s.media_present === false ? R.badge('media missing', 'fail') : null)));
      });
      card.appendChild(ss);

      // clips
      var cs = h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, 'Clips'), h('a', { class: 'small', onclick: function () { R.go('clips'); } }, 'open Clips')));
      if (!clips.length) cs.appendChild(h('div', { class: 'muted small' }, 'No clips yet.'));
      else cs.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl compact clipstatus' },
        h('thead', null, h('tr', null, ['clip', 'length', 'segs', 'rev', 'render', 'QC', 'brief', 'export', ''].map(function (x) { return h('th', null, x); }))),
        h('tbody', null, clips.map(function (c) {
          var lr = c.last_render;
          var rb = !c.renders ? R.badge('none', 'none') : (c.edits_newer_than_render ? R.badge('stale', 'warn', c.stale_reason || 'edited after the render') : R.badge(c.renders + ' · ' + (lr && lr.quality ? lr.quality : 'ok'), 'pass', lr ? R.pretty(lr) : ''));
          return h('tr', { 'data-clip': c.id },
            h('td', null, h('b', null, c.name || c.id)), h('td', { class: 'mono small' }, R.fmt(c.duration)), h('td', { class: 'small' }, String(c.segments)), h('td', { class: 'mono small' }, String(c.revision)),
            h('td', null, rb),
            h('td', null, c.qc ? R.statusBadge(qcStatus(c.qc), qcStatus(c.qc)) : R.badge('-', 'none')),
            h('td', null, c.brief_check ? R.statusBadge(briefStatus(c.brief_check), briefStatus(c.brief_check)) : R.badge(c.has_brief ? 'unchecked' : '-', 'none')),
            h('td', null, c.exported ? R.badge(c.exported_older_render ? 'old render' : 'exported', c.exported_older_render ? 'warn' : 'pass') : R.badge('-', 'none')),
            h('td', null, R.button('Edit', function () { R.go('editor', c.id); }, { small: true })));
        })))));
      card.appendChild(cs);
      (d.warnings || []).forEach(function (w) { card.appendChild(h('div', { class: 'small st-warn' }, w)); });
    });
  }
  function stat(n, label) { return h('div', { class: 'stat' }, h('b', null, String(n)), h('span', null, label)); }
  function qcStatus(q) { if (typeof q === 'string') return q; if (!q) return '-'; if (q.passed === true) return 'pass'; if (q.passed === false) return 'fail'; return q.status || q.verdict || 'checked'; }
  function briefStatus(b) { if (typeof b === 'string') return b; return (b && (b.status || b.verdict)) || 'checked'; }

  // ------------------------------------------------------------ recent projects
  function recent() {
    var card = h('div', { class: 'card' });
    var body = h('div');
    card.appendChild(h('div', { class: 'row' }, h('h2', null, 'Recent projects'), h('span', { class: 'grow' }),
      R.button('Remove missing', function () { return load(true); }, { small: true, cls: 'ghost', title: 'project.list --prune-missing true' })));
    card.appendChild(body);
    function load(prune) {
      return R.run('project.list', prune ? { prune_missing: true } : {}, true).then(function (d) {
        R.clear(body);
        var list = (d && d.projects) || [];
        if (!list.length) { body.appendChild(h('div', { class: 'muted small' }, 'No projects recorded yet.')); return; }
        body.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl compact recent' }, h('tbody', null, list.slice(0, 15).map(function (p) {
          var cur = S.project && p.path.toLowerCase() === S.project.replace(/[\\/]+$/, '').toLowerCase();
          return h('tr', { class: cur ? 'sel' : '' },
            h('td', null, h('b', null, p.name), h('div', { class: 'muted tiny mono ell', style: { maxWidth: '330px' }, title: p.path }, p.path)),
            h('td', { class: 'small nowrap' }, p.sources + ' src · ' + p.clips + ' clips'),
            h('td', { class: 'small muted nowrap' }, String(p.last_opened || '').replace('T', ' ').slice(0, 16)),
            h('td', null, p.exists ? (cur ? h('span', { class: 'muted small' }, 'open') : R.button('Open', function () { return openDir(p.path); }, { small: true, primary: true })) : R.badge('missing', 'fail')));
        })))));
      });
    }
    load(false);
    return card;
  }

  // ------------------------------------------------------------ pack / unpack
  function packCard() {
    var card = h('div', { class: 'card packcard' }, h('h2', null, 'Pack / unpack'));
    if (R.has('project.pack')) {
      var out = h('input', { type: 'text', class: 'grow', placeholder: 'default: next to the project, <name>.rustler.zip' });
      var media = h('input', { type: 'checkbox' }), analysis = h('input', { type: 'checkbox', checked: true }), renders = h('input', { type: 'checkbox', checked: true });
      var res = h('div');
      card.appendChild(h('div', { class: 'small muted' }, 'Zip the project to move or share it (the original is not touched).'));
      card.appendChild(h('div', { class: 'row' }, out));
      card.appendChild(h('div', { class: 'row' }, h('label', { class: 'chk small' }, analysis, ' analysis'), h('label', { class: 'chk small' }, renders, ' renders + exports'), h('label', { class: 'chk small' }, media, ' source media'),
        h('span', { class: 'grow' }),
        R.button('Pack project', function () {
          var i = { include_media: media.checked, include_analysis: analysis.checked, include_renders: renders.checked };
          if (out.value.trim()) i.out = out.value.trim();
          R.clear(res).appendChild(h('div', { class: 'muted small' }, 'packing...'));
          return R.jobs.start('project.pack', i).then(function (jid) {
            if (!jid) { R.clear(res); return; }
            R.waitJob(jid, function (d) { R.clear(res).appendChild(packView(d)); });
          });
        }, { small: true, primary: true })));
      card.appendChild(res);
    }
    if (R.has('project.unpack')) {
      var arch = h('input', { type: 'text', class: 'grow', placeholder: 'C:\\share\\proj.rustler.zip' });
      var dir = h('input', { type: 'text', class: 'grow', placeholder: 'C:\\work\\proj (new folder)' });
      var force = h('input', { type: 'checkbox' });
      var ures = h('div');
      card.appendChild(h('div', { class: 'isec' }, h('div', { class: 'ititle' }, h('span', null, 'Unpack a .rustler.zip'))));
      card.appendChild(h('div', { class: 'row' }, R.field('Archive', arch)));
      card.appendChild(h('div', { class: 'row' }, R.field('Into folder', dir)));
      card.appendChild(h('div', { class: 'row' }, h('label', { class: 'chk small' }, force, ' overwrite an existing project'), h('span', { class: 'grow' }),
        R.button('Unpack', function () {
          if (!arch.value.trim() || !dir.value.trim()) { R.toast('Enter the archive and a target folder', 'bad'); return; }
          return R.run('project.unpack', { archive: arch.value.trim().replace(/^"|"$/g, ''), dir: dir.value.trim().replace(/^"|"$/g, ''), force: force.checked }).then(function (d) {
            if (!d) return;
            var target = d.project_dir || d.dir || dir.value.trim();
            R.clear(ures).appendChild(h('div', { class: 'small' }, R.badge('unpacked', 'pass'), ' ', h('span', { class: 'mono' }, target), ' ',
              R.button('Open it', function () { return openDir(target); }, { small: true, primary: true })));
            ures.appendChild(h('details', { class: 'raw' }, h('summary', null, 'project.unpack result'), h('pre', null, R.pretty(d))));
          });
        }, { small: true })));
      card.appendChild(ures);
    }
    return card;
  }
  function packView(d) {
    var size = d.bytes || d.size_bytes || d.size;
    return h('div', { class: 'small' }, R.badge('packed', 'pass'), ' ', h('span', { class: 'mono' }, d.archive || d.out || d.path || ''),
      size ? ' · ' + (size / 1048576).toFixed(1) + ' MB' : '', d.files !== undefined ? ' · ' + (Array.isArray(d.files) ? d.files.length : d.files) + ' files' : '',
      h('details', { class: 'raw' }, h('summary', null, 'project.pack result'), h('pre', null, R.pretty(d))));
  }

  R.views.project = { label: 'Project', icon: '\u25A3', render: render };
})();
