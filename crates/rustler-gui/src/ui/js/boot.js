// Boot: load the command list (feature detection), the project, then route from the URL hash.
(function () {
  'use strict';
  var R = window.R, S = R.S;
  function boot() {
    var hash = (location.hash || '').replace('#', '');
    var parts = hash.split('/');
    var initialView = parts[0], initialArg = parts[1] ? decodeURIComponent(parts[1]) : '';
    window.addEventListener('error', function (e) { R.toast('UI error: ' + e.message, 'bad'); });
    window.addEventListener('unhandledrejection', function (e) { R.toast('UI error: ' + e.reason, 'bad'); });
    var saved = R.store('rustler.project');
    if (saved) S.project = saved;
    R.buildNav();
    R.getJson('/api/commands?t=' + encodeURIComponent(R.T)).then(function (list) {
      (Array.isArray(list) ? list : []).forEach(function (c) { S.cmds[c.name] = c; });
      return R.refreshProject(true);
    }).then(function () {
      R.jobs.poll();
      var ok = !!R.views[initialView];
      R.go(ok ? initialView : 'project', ok ? initialArg : '');
    });
  }
  boot();
})();
