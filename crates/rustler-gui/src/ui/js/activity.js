// Activity page: live feed of events.ndjson (agent and human actions).
(function () {
  'use strict';
  var R = window.R, h = R.h, S = R.S;

  R.eventRow = function (e) {
    var actor = String(e.actor || 'system').toLowerCase();
    var time = String(e.ts || '').replace('T', ' ').slice(0, 19);
    return h('div', { class: 'ev' },
      h('span', { class: 'muted mono small' }, time),
      h('span', { class: 'badge ' + actor }, actor),
      h('strong', null, e.kind || ''),
      h('span', { class: 'd', title: R.short(e.data, 400) }, R.short(e.data, 160)));
  };

  function render(v) {
    v.appendChild(h('h1', null, 'Activity'));
    v.appendChild(h('p', { class: 'muted small' }, 'Live feed of events.ndjson (agent and human actions), refreshed every 2 s.'));
    var box = h('div', { class: 'card' });
    var jobsBox = h('div');
    v.appendChild(jobsBox);
    v.appendChild(box);
    function tick() {
      R.getJson(R.apiUrl('/api/events')).then(function (evs) {
        if (S.view !== 'activity' || !Array.isArray(evs)) return;
        R.clear(box);
        if (!evs.length) { box.appendChild(h('p', { class: 'muted' }, 'No events yet.')); return; }
        evs.slice().reverse().forEach(function (e) { box.appendChild(R.eventRow(e)); });
      });
    }
    R.onLeave(R.jobs.on(function (list) { drawJobs(jobsBox, list); }));
    R.jobs.poll();
    tick();
    R.every(2000, tick);
  }
  function drawJobs(box, list) {
    R.clear(box);
    if (!list.length) return;
    box.appendChild(h('div', { class: 'card' }, h('h2', null, 'Background jobs (this GUI session)'), R.jobTable(list.slice().reverse().slice(0, 15))));
  }
  R.jobTable = function (list) {
    return h('table', { class: 'tbl' }, h('tbody', null, list.map(function (j) {
      var p = j.progress || {};
      var pct = j.status === 'ok' ? 1 : (p.pct || 0);
      var secs = ((j.finished_ms || Date.now()) - j.started_ms) / 1000;
      return h('tr', null,
        h('td', { class: 'mono' }, j.command),
        h('td', null, h('div', { class: 'pbar' + (j.status === 'error' ? ' bad' : '') }, h('i', { style: { width: Math.round(pct * 100) + '%' } }))),
        h('td', { class: 'st-' + (j.status === 'ok' ? 'pass' : j.status === 'running' ? 'run' : 'fail') }, j.status + (j.status === 'running' && p.stage ? ' · ' + p.stage : '')),
        h('td', { class: 'muted small' }, secs.toFixed(0) + ' s'),
        h('td', null, j.status === 'running' ? R.button('Cancel', function () { return R.jobs.cancel(j.id); }, { small: true }) : null));
    })));
  };
  R.views.activity = { label: 'Activity', icon: '≡', render: render };
})();
