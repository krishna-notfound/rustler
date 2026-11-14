// Schema-driven forms: any registry command becomes a small form from its JSON input schema.
// Used for commands whose UI is generic (overlays, audio fx, effects, brief, variants, assets...).
(function () {
  'use strict';
  var R = window.R, h = R.h;

  function resolve(schema, root) {
    var guard = 0;
    while (schema && schema.$ref && guard++ < 10) {
      var name = String(schema.$ref).split('/').pop();
      var defs = root.$defs || root.definitions || {};
      schema = defs[name] || {};
    }
    if (schema && Array.isArray(schema.allOf) && schema.allOf.length === 1) return resolve(schema.allOf[0], root);
    return schema || {};
  }
  // -> {kind: 'string'|'number'|'integer'|'boolean'|'enum'|'array'|'json', options?, nullable}
  function kindOf(p, root) {
    p = resolve(p, root);
    var nullable = false;
    var alts = p.anyOf || p.oneOf;
    if (Array.isArray(alts)) {
      var nonNull = alts.filter(function (a) { return !(a && a.type === 'null'); });
      nullable = nonNull.length < alts.length;
      // enum expressed as oneOf of consts / single-value enums
      var consts = [];
      var allConst = nonNull.every(function (a) {
        a = resolve(a, root);
        if (a.const !== undefined) { consts.push(a.const); return true; }
        if (Array.isArray(a.enum)) { consts = consts.concat(a.enum); return true; }
        return false;
      });
      if (allConst && consts.length) return { kind: 'enum', options: consts, nullable: nullable };
      if (nonNull.length === 1) { var k = kindOf(nonNull[0], root); k.nullable = nullable || k.nullable; return k; }
      return { kind: 'json', nullable: nullable };
    }
    var t = p.type;
    if (Array.isArray(t)) { nullable = t.indexOf('null') >= 0; t = t.filter(function (x) { return x !== 'null'; })[0]; }
    if (Array.isArray(p.enum)) return { kind: 'enum', options: p.enum.filter(function (x) { return x !== null; }), nullable: nullable };
    if (t === 'string') return { kind: 'string', nullable: nullable };
    if (t === 'number' || t === 'integer') return { kind: t, nullable: nullable, min: p.minimum, max: p.maximum };
    if (t === 'boolean') return { kind: 'boolean', nullable: nullable };
    if (t === 'array') {
      var it = resolve(p.items || {}, root);
      if (it.type === 'string' || it.type === 'number' || it.type === 'integer') return { kind: 'array', item: it.type, nullable: nullable };
      return { kind: 'json', nullable: nullable };
    }
    return { kind: 'json', nullable: nullable };
  }
  R.schemaKind = function (cmd, prop) {
    var s = R.spec(cmd); if (!s) return null;
    var root = s.input_schema || {};
    var p = (root.properties || {})[prop]; if (!p) return null;
    return kindOf(p, root);
  };

  // Builds a form for `cmd`. opts: {fixed: {k: v} (hidden, always sent), initial: {k: v}, hide: [k],
  // submit: label, done(data, input), compact, fields: [only these keys]}
  R.schemaForm = function (cmd, opts) {
    var o = opts || {};
    var spec = R.spec(cmd);
    if (!spec) return { el: R.na(cmd), values: function () { return {}; } };
    var root = spec.input_schema || {};
    var props = root.properties || {};
    var req = root.required || [];
    var fixed = o.fixed || {};
    var hide = (o.hide || []).concat(['base_revision', 'expect_revision', 'dry_run']);
    var getters = {};
    var rows = [];
    // required fields first, then text-like fields, then the rest (schema order)
    var keys = Object.keys(props);
    var rank = function (k) { return (req.indexOf(k) >= 0 ? 0 : 2) + (/^(text|name|path|kind|style)$/.test(k) ? 0 : 1); };
    keys = keys.map(function (k, i) { return [k, i]; }).sort(function (a, b) { return rank(a[0]) - rank(b[0]) || a[1] - b[1]; }).map(function (x) { return x[0]; });
    keys.forEach(function (k) {
      if (Object.prototype.hasOwnProperty.call(fixed, k) || hide.indexOf(k) >= 0) return;
      if (o.fields && o.fields.indexOf(k) < 0) return;
      var p = resolve(props[k], root);
      var kd = kindOf(props[k], root);
      var init = o.initial && o.initial[k] !== undefined ? o.initial[k] : (p.default !== undefined ? p.default : undefined);
      var el, get;
      var required = req.indexOf(k) >= 0;
      if (kd.kind === 'enum') {
        var opts2 = (required ? [] : [['', '(default)']]).concat(kd.options.map(function (x) { return [String(x), String(x)]; }));
        el = R.select(opts2, init === undefined ? '' : init);
        get = function () { if (el.value === '') return undefined; var x = kd.options.filter(function (v) { return String(v) === el.value; })[0]; return x === undefined ? el.value : x; };
      } else if (kd.kind === 'boolean') {
        if (required) {
          el = h('input', { type: 'checkbox', checked: !!init });
          get = function () { return el.checked; };
        } else {
          el = R.select([['', '(default)'], ['true', 'yes'], ['false', 'no']], init === undefined ? '' : String(init));
          get = function () { return el.value === '' ? undefined : el.value === 'true'; };
        }
      } else if (kd.kind === 'number' || kd.kind === 'integer') {
        el = h('input', { type: 'number', step: kd.kind === 'integer' ? '1' : 'any', value: init === undefined ? '' : init, class: 'num' });
        get = function () { return el.value === '' ? undefined : Number(el.value); };
      } else if (kd.kind === 'array') {
        el = h('input', { type: 'text', placeholder: 'comma separated', value: Array.isArray(init) ? init.join(', ') : '' });
        get = function () {
          var parts = el.value.split(',').map(function (x) { return x.trim(); }).filter(Boolean);
          if (!parts.length) return undefined;
          return kd.item === 'string' ? parts : parts.map(Number);
        };
      } else if (kd.kind === 'string') {
        var long = /text|notes|description|script|body/.test(k);
        el = long ? h('textarea', { rows: 2, class: 'plain' }) : h('input', { type: 'text' });
        if (init !== undefined) el.value = init;
        get = function () { return el.value === '' ? undefined : el.value; };
      } else {
        el = h('textarea', { rows: 2, placeholder: 'JSON', spellcheck: 'false' });
        if (init !== undefined) el.value = JSON.stringify(init);
        get = function () {
          if (!el.value.trim()) return undefined;
          try { return JSON.parse(el.value); } catch (e) { throw new Error(k + ': invalid JSON'); }
        };
      }
      getters[k] = get;
      rows.push(h('label', { class: 'field' + (kd.kind === 'json' || el.tagName === 'TEXTAREA' ? ' wide' : '') },
        h('span', { class: 'flabel' }, k.replace(/_/g, ' ') + (required ? ' *' : '')), el,
        p.description ? h('span', { class: 'fhint' }, p.description) : null));
    });
    function values() {
      var out = Object.assign({}, fixed);
      Object.keys(getters).forEach(function (k) { var v = getters[k](); if (v !== undefined) out[k] = v; });
      return out;
    }
    var el = h('div', { class: 'sform' + (o.compact ? ' compact' : '') }, h('div', { class: 'fgrid' }, rows));
    if (o.submit !== false) {
      el.appendChild(h('div', { class: 'row' }, R.button(o.submit || spec.name, function () {
        var input;
        try { input = values(); } catch (e) { R.toast(String(e.message || e), 'bad'); return; }
        var p = o.run ? o.run(cmd, input) : R.run(cmd, input);
        return Promise.resolve(p).then(function (d) { if (d && o.done) return o.done(d, input); });
      }, { primary: true }), spec.summary ? h('span', { class: 'muted small' }, spec.summary) : null));
    }
    return { el: el, values: values };
  };

  // A result viewer: renders a JSON-ish value as a compact table/list with any images shown.
  R.resultView = function (d) {
    var box = h('div', { class: 'result' });
    if (d === null || d === undefined) return box;
    var images = [];
    // image paths only (not sentences that happen to end in ".jpg")
    JSON.stringify(d, function (k, v) { if (typeof v === 'string' && /\.(jpe?g|png|webp)$/i.test(v) && !/[;:,]\s|\.\s/.test(v) && images.indexOf(v) < 0) images.push(v); return v; });
    var arr = Array.isArray(d) ? d : R.listOf(d, 'texts', 'events', 'spans', 'scenes', 'candidates', 'suggestions', 'variants', 'items', 'matches', 'results');
    if (arr.length && typeof arr[0] === 'object') {
      var cols = [];
      arr.slice(0, 50).forEach(function (r) { Object.keys(r || {}).forEach(function (k) { if (cols.indexOf(k) < 0 && cols.length < 7) cols.push(k); }); });
      box.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl' }, h('thead', null, h('tr', null, cols.map(function (c) { return h('th', null, c); }))),
        h('tbody', null, arr.slice(0, 200).map(function (r) {
          return h('tr', null, cols.map(function (c) { var v = r[c]; return h('td', { class: typeof v === 'number' ? 'mono' : '' }, typeof v === 'number' ? R.round(v, 3) : R.short(v === undefined ? '' : v, 80)); }));
        })))));
    }
    images.slice(0, 4).forEach(function (p) { box.appendChild(R.img(p)); });
    box.appendChild(h('details', { class: 'raw' }, h('summary', null, 'raw result'), h('pre', null, R.pretty(d))));
    return box;
  };
})();
