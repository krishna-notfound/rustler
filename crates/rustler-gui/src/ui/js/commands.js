// Commands page: every registry command as a JSON form (the same input the CLI takes).
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  function render(v) {
    v.appendChild(h('h1', null, 'Commands'));
    var names = Object.keys(S.cmds).sort();
    var filter = h('input', { class: 'grow', placeholder: 'filter ' + names.length + ' commands' });
    var box = h('div');
    v.appendChild(h('div', { class: 'row' }, filter));
    v.appendChild(h('p', { class: 'muted small' }, 'Every registry command is available here as a JSON form, the same input the CLI takes.'));
    v.appendChild(box);
    function draw() {
      R.clear(box);
      var f = filter.value.trim().toLowerCase();
      names.filter(function (n) { return !f || n.indexOf(f) >= 0 || String(S.cmds[n].summary || '').toLowerCase().indexOf(f) >= 0; })
        .forEach(function (n) { box.appendChild(commandItem(S.cmds[n])); });
    }
    filter.addEventListener('input', draw);
    draw();
  }
  function commandItem(spec) {
    var ta = h('textarea', { spellcheck: 'false' });
    ta.value = '{}';
    var out = h('div');
    var props = (spec.input_schema && spec.input_schema.properties) || {};
    var req = (spec.input_schema && spec.input_schema.required) || [];
    var rows = Object.keys(props).map(function (k) {
      var p = props[k] || {};
      var ty = p.type ? (Array.isArray(p.type) ? p.type.join('|') : p.type) : (p.$ref ? String(p.$ref).split('/').pop() : '');
      return h('tr', null, h('td', { class: 'mono' }, k + (req.indexOf(k) >= 0 ? ' *' : '')), h('td', { class: 'mono' }, ty), h('td', { class: 'muted' }, p.description || ''));
    });
    var a = spec.annotations || {};
    var flags = Object.keys(a).filter(function (k) { return a[k]; }).join(', ');
    var examples = (spec.examples || []).map(function (e) { return h('div', { class: 'small muted' }, e.description + ': ', h('span', { class: 'mono' }, e.cli)); });
    return h('details', null,
      h('summary', null, h('span', { class: 'n' }, spec.name), h('span', { class: 'muted' }, spec.summary)),
      h('div', { class: 'body' },
        flags ? h('div', { class: 'small muted' }, 'flags: ' + flags) : null,
        (spec.positional && spec.positional.length) ? h('div', { class: 'small muted' }, 'cli positional: ' + spec.positional.join(' ')) : null,
        rows.length ? h('table', null, h('thead', null, h('tr', null, h('th', null, 'input field (* required)'), h('th', null, 'type'), h('th', null, ''))), h('tbody', null, rows)) : null,
        examples,
        ta,
        h('div', { class: 'row' }, R.button('Run', function () {
          var parsed;
          try { parsed = JSON.parse(ta.value || '{}'); } catch (e) { R.toast('Invalid JSON: ' + e.message, 'bad'); return; }
          var long = spec.annotations && spec.annotations.long_running;
          if (long) {
            return R.jobs.start(spec.name, parsed).then(function (id) {
              if (!id) return;
              R.clear(out).appendChild(h('div', { class: 'muted small' }, 'Running in the background as ' + id + ' (see Activity).'));
              var off = R.jobs.on(function (list) {
                var j = list.filter(function (x) { return x.id === id; })[0];
                if (!j || j.status === 'running') return;
                off();
                R.clear(out).appendChild(j.result && j.result.ok ? R.resultView(j.result.data) : h('pre', null, R.pretty(j.result)));
              });
            });
          }
          return R.call(spec.name, parsed).then(function (env) {
            R.clear(out).appendChild(env && env.ok ? R.resultView(env.data) : h('pre', { class: 'err' }, R.pretty(env)));
          });
        }, { primary: true }), spec.annotations && spec.annotations.long_running ? h('span', { class: 'muted small' }, 'long-running: runs as a background job') : null),
        out));
  }
  R.views.commands = { label: 'Commands', icon: '⌘', render: render };
})();
