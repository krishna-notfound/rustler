// Brief page: edit the job brief (brief.set|show|clear), check clips against it (brief.check) with
// one-click fixes, and draft the post text (brief.description) with a copy button.
// R.briefPanel(el, clipId, mutate) is the same check/description UI for the editor's Brief tab.
(function () {
  'use strict';
  var R = window.R, h = R.h;

  function lines(v) { return (Array.isArray(v) ? v : []).join('\n'); }
  function split(s) { return String(s || '').split(/\n|,/).map(function (x) { return x.trim(); }).filter(Boolean); }

  // ------------------------------------------------------------ brief form
  function briefForm(box, onSaved) {
    R.clear(box).appendChild(h('span', { class: 'muted small' }, 'loading brief...'));
    return Promise.all([R.run('brief.show', {}, true), R.loadStyleDefs(), R.has('layout.presets') ? R.run('layout.presets', {}, true) : null]).then(function (res) {
      var d = res[0] || {}, b = d.brief || {}, styles = res[1] || [], lp = res[2] || {};
      var layouts = (lp.built_in || []).map(function (p) { return p.name; }).concat((lp.custom || []).map(function (p) { return p.name; }));
      var pay = b.payout || {};
      var f = {
        creator: h('input', { type: 'text', value: b.creator || '', placeholder: '@creator' }),
        campaign: h('input', { type: 'text', value: b.campaign || '' }),
        platform: R.select([['', '(any)'], ['tiktok', 'TikTok'], ['reels', 'Reels'], ['shorts', 'Shorts'], ['all', 'all']], b.platform || ''),
        min_duration: h('input', { type: 'number', step: '0.5', min: 0, class: 'num', value: b.min_duration === null || b.min_duration === undefined ? '' : b.min_duration }),
        max_duration: h('input', { type: 'number', step: '0.5', min: 0, class: 'num', value: b.max_duration === null || b.max_duration === undefined ? '' : b.max_duration }),
        aspect: h('input', { type: 'text', value: b.aspect || '', placeholder: '9:16' }),
        caption_style: R.select([['', '(any)']].concat(styles.map(function (s) { return [s.name, s.name + (s.built_in ? '' : ' (custom)')]; })), b.caption_style || ''),
        layout: R.select([['', '(any)']].concat(layouts.map(function (n) { return [n, n]; })), b.layout || ''),
        required_text: h('textarea', { rows: 2, class: 'plain', placeholder: 'one phrase per line' }),
        banned_words: h('textarea', { rows: 2, class: 'plain', placeholder: 'one per line' }),
        required_hashtags: h('input', { type: 'text', value: (b.required_hashtags || []).join(', '), placeholder: '#ad, #clip' }),
        required_mentions: h('input', { type: 'text', value: (b.required_mentions || []).join(', '), placeholder: '@brand' }),
        credit_text: h('input', { type: 'text', value: b.credit_text || '', placeholder: 'Clip from @creator (must be in an overlay)' }),
        watermark_text: h('input', { type: 'text', value: b.watermark_text || '' }),
        pay_model: R.select([['', '(none)'], ['per_view', 'per view'], ['per_clip', 'per clip']], pay.model || ''),
        pay_rate: h('input', { type: 'number', step: 'any', min: 0, class: 'num', value: pay.rate === null || pay.rate === undefined ? '' : pay.rate }),
        pay_currency: h('input', { type: 'text', value: pay.currency || '', placeholder: 'USD', class: 'short' }),
        notes: h('textarea', { rows: 3, class: 'plain' })
      };
      f.required_text.value = lines(b.required_text);
      f.banned_words.value = lines(b.banned_words);
      f.notes.value = b.notes || '';
      if (b.caption_style && !styles.some(function (s) { return s.name === b.caption_style; })) f.caption_style.appendChild(R.option(b.caption_style, b.caption_style, true));
      if (b.layout && layouts.indexOf(b.layout) < 0) f.layout.appendChild(R.option(b.layout, b.layout, true));
      function input() {
        var num = function (el) { return el.value === '' ? undefined : Number(el.value); };
        var str = function (el) { return el.value.trim() || undefined; };
        var i = { replace: true, creator: str(f.creator), campaign: str(f.campaign), platform: f.platform.value || undefined, min_duration: num(f.min_duration), max_duration: num(f.max_duration),
          aspect: str(f.aspect), caption_style: f.caption_style.value || undefined, layout: f.layout.value || undefined,
          required_text: split(f.required_text.value), banned_words: split(f.banned_words.value),
          required_hashtags: split(f.required_hashtags.value), required_mentions: split(f.required_mentions.value),
          credit_text: str(f.credit_text), watermark_text: str(f.watermark_text), notes: str(f.notes) };
        if (f.pay_model.value) i.payout = { model: f.pay_model.value, rate: num(f.pay_rate), currency: str(f.pay_currency) };
        Object.keys(i).forEach(function (k) { if (i[k] === undefined) delete i[k]; });
        if (i.min_duration !== undefined && i.max_duration !== undefined && i.min_duration > i.max_duration) throw new Error('min duration is larger than max duration');
        return i;
      }
      R.clear(box);
      box.appendChild(h('div', { class: 'briefform' },
        h('div', { class: 'fgrid3' }, R.field('Creator', f.creator), R.field('Campaign', f.campaign), R.field('Platform', f.platform),
          R.field('Min duration (s)', f.min_duration), R.field('Max duration (s)', f.max_duration), R.field('Aspect', f.aspect, 'W:H; Rustler renders 9:16'),
          R.field('Caption style', f.caption_style), R.field('Layout', f.layout), R.field('Credit text', f.credit_text, 'an overlay must contain it'),
          R.field('Watermark text', f.watermark_text), R.field('Hashtags', f.required_hashtags, 'comma separated'), R.field('Mentions', f.required_mentions, 'comma separated')),
        h('div', { class: 'fgrid2' }, R.field('Required text', f.required_text, 'must appear in captions, overlays or speech'), R.field('Banned words', f.banned_words, 'must not appear anywhere')),
        h('div', { class: 'row' }, R.field('Payout', f.pay_model), R.field('Rate', f.pay_rate), R.field('Currency', f.pay_currency), h('span', { class: 'grow' })),
        R.field('Notes', f.notes),
        h('div', { class: 'row' },
          R.button('Save brief', function () {
            var i; try { i = input(); } catch (e) { R.toast(e.message, 'bad'); return; }
            return R.run('brief.set', i).then(function (r) { if (r) { R.toast('Brief saved (' + String(r.path).split(/[\\/]/).pop() + ')', 'good'); briefForm(box, onSaved); if (onSaved) onSaved(r); } });
          }, { primary: true, need: ['brief.set'] }),
          d.exists && R.has('brief.clear') ? R.button('Clear brief', function () {
            return R.confirm('Clear brief', 'Delete the project brief (brief.json)?', 'Clear').then(function (ok) {
              if (ok) return R.run('brief.clear', {}).then(function (r) { if (r) { R.toast('Brief cleared', 'good'); briefForm(box, onSaved); if (onSaved) onSaved(null); } });
            });
          }, { cls: 'ghost danger' }) : null,
          h('span', { class: 'muted small' }, d.exists ? 'Saving replaces the whole brief with this form (brief.set --replace true).' : 'No brief yet: fill in what the job asks for and save.'),
          b.updated_at ? h('span', { class: 'muted small' }, 'updated ' + String(b.updated_at).replace('T', ' ').slice(0, 16)) : null)));
    });
  }

  // ------------------------------------------------------------ check + description
  // Renders a brief.check result: overall status, rules with fix buttons, post items.
  function checkView(d, opts) {
    var c = d.check || d;
    var box = h('div', { class: 'briefcheck' });
    box.appendChild(h('div', { class: 'row' }, R.statusBadge(c.status, 'brief ' + c.status), h('b', null, c.clip_name || c.clip_id),
      h('span', { class: 'muted small' }, 'rev ' + c.clip_revision + ' · ' + String(c.checked_at || '').replace('T', ' ').slice(0, 19))));
    var rows = (c.rules || []).map(function (r) {
      return h('tr', { class: 'st-row-' + r.status },
        h('td', null, R.statusBadge(r.status)), h('td', { class: 'mono small' }, r.rule), h('td', { class: 'small' }, r.message),
        h('td', null, (r.fix || []).map(function (line) { return R.cliRow(line, opts); })));
    });
    box.appendChild(h('div', { class: 'tablewrap' }, h('table', { class: 'tbl compact' }, h('thead', null, h('tr', null, h('th', null, ''), h('th', null, 'rule'), h('th', null, 'result'), h('th', null, 'fix'))), h('tbody', null, rows))));
    if ((c.post_items || []).length) box.appendChild(h('div', { class: 'small' }, h('b', null, 'For the post: '), c.post_items.join(' · ')));
    (c.warnings || []).forEach(function (w) { box.appendChild(h('div', { class: 'small st-warn' }, w)); });
    return box;
  }
  function descView(d) {
    var x = d.description || d;
    var ta = h('textarea', { rows: 9, class: 'plain desc', readonly: true });
    ta.value = x.text || '';
    return h('div', { class: 'descbox' },
      h('div', { class: 'row' }, h('b', null, 'Post text'), h('span', { class: 'muted small' }, (x.platform || '') + ' · ' + (x.chars !== undefined ? x.chars : (x.text || '').length) + ' chars'),
        h('span', { class: 'grow' }), R.button('Copy', function () { return R.copyText(ta.value); }, { small: true, primary: true })),
      ta,
      x.hook ? h('div', { class: 'small muted' }, 'hook: ', x.hook) : null);
  }
  R.briefCheckView = checkView;
  R.briefDescView = descView;

  // Editor tab / per-clip block.
  R.briefPanel = function (el, clipId, mutate, onChange) {
    var out = h('div'), desc = h('div');
    var opts = { mutate: mutate, done: function () { if (onChange) onChange(); } };
    var summary = h('div', { class: 'small' });
    el.appendChild(h('div', { class: 'row' }, summary, h('span', { class: 'grow' }), R.button('Edit brief', function () { R.go('brief'); }, { small: true, cls: 'ghost' })));
    R.run('brief.show', {}, true).then(function (d) {
      var b = d && d.brief;
      if (!b) { summary.appendChild(h('span', { class: 'muted' }, 'No brief for this project yet.')); return; }
      var bits = [];
      if (b.platform) bits.push(b.platform);
      if (b.min_duration || b.max_duration) bits.push((b.min_duration || 0) + '–' + (b.max_duration || '∞') + ' s');
      if (b.caption_style) bits.push('style ' + b.caption_style);
      if (b.layout) bits.push('layout ' + b.layout);
      if ((b.required_text || []).length) bits.push(b.required_text.length + ' required phrase(s)');
      if ((b.banned_words || []).length) bits.push(b.banned_words.length + ' banned');
      if (b.credit_text) bits.push('credit overlay');
      summary.appendChild(h('span', null, h('b', null, b.campaign || b.creator || 'Brief'), ': ', bits.join(' · ') || 'no rules'));
    });
    function check() {
      return R.run('brief.check', { clip_id: clipId }).then(function (d) { if (d) R.clear(out).appendChild(checkView(d, opts)); });
    }
    opts.done = function () { if (onChange) onChange(); return check(); };
    el.appendChild(h('div', { class: 'row' },
      R.button('Check against brief', check, { small: true, primary: true, need: ['brief.check'] }),
      R.button('Post description', function () { return R.run('brief.description', { clip_id: clipId }).then(function (d) { if (d) R.clear(desc).appendChild(descView(d)); }); }, { small: true, need: ['brief.description'] })));
    el.appendChild(out);
    el.appendChild(desc);
    // last saved check, if any
    R.getJson(R.optUrl('clips/' + clipId + '/brief_check.json')).then(function (c) {
      if (c && c.rules && !out.firstChild) { out.appendChild(h('div', { class: 'muted small' }, 'Last saved check:')); out.appendChild(checkView(c, opts)); }
    });
  };

  // ------------------------------------------------------------ page
  function render(v) {
    v.appendChild(h('div', { class: 'pagehead' }, h('h1', null, 'Brief'), h('span', { class: 'muted' }, 'The job’s rules: length, style, required and banned text, credit, hashtags, payout.')));
    if (!R.has('brief.show')) { v.appendChild(R.na('brief.show', 'brief.set')); return; }
    var formBox = h('div');
    var clipsBox = h('div');
    v.appendChild(h('div', { class: 'card' }, h('h2', null, 'Brief'), formBox));
    v.appendChild(h('div', { class: 'card' }, h('div', { class: 'row' }, h('h2', null, 'Clips vs brief'), h('span', { class: 'grow' }),
      R.button('Check all clips', function () { return checkAll(); }, { small: true, primary: true, need: ['brief.check'] })), clipsBox));
    var rows = {};
    function drawClips() {
      R.clear(clipsBox);
      return R.loadClips().then(function (clips) {
        if (!clips.length) { clipsBox.appendChild(h('p', { class: 'muted' }, 'No clips yet.')); return; }
        clips.forEach(function (c) {
          var id = R.cid(c);
          var res = h('div', { class: 'clipcheck' }), desc = h('div');
          var head = h('div', { class: 'row' }, h('b', null, c.name || id), h('span', { class: 'muted small mono' }, id), h('span', { class: 'grow' }),
            R.button('Check', function () { return one(id); }, { small: true, need: ['brief.check'] }),
            R.button('Post text', function () { return R.run('brief.description', { clip_id: id }).then(function (d) { if (d) R.clear(desc).appendChild(descView(d)); }); }, { small: true, need: ['brief.description'] }),
            R.button('Open editor', function () { R.go('editor', id); }, { small: true, cls: 'ghost' }));
          rows[id] = res;
          clipsBox.appendChild(h('div', { class: 'cliprow' }, head, res, desc));
          R.getJson(R.optUrl('clips/' + id + '/brief_check.json')).then(function (ck) { if (ck && ck.rules && !res.firstChild) res.appendChild(checkView(ck, { done: function () { return one(id); } })); });
        });
      });
    }
    function one(id) {
      return R.run('brief.check', { clip_id: id }).then(function (d) { if (d && rows[id]) R.clear(rows[id]).appendChild(checkView(d, { done: function () { return one(id); } })); });
    }
    function checkAll() {
      var ids = Object.keys(rows);
      return ids.reduce(function (p, id) { return p.then(function () { return one(id); }); }, Promise.resolve());
    }
    briefForm(formBox, function () { drawClips(); });
    drawClips();
  }
  R.views.brief = { label: 'Brief', icon: '☰', render: render };
})();
