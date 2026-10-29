/* Nock landing page: the interactive notch demo and page motion. Vanilla JS, no dependencies. */
(function () {
  'use strict';
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var RM = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  var notch = $('#notch'), pill = $('#pill'), pillText = $('#pillText'), panel = $('#panel');
  var K = 1; // scale so the 600px panel fits narrow screens

  /* ---------- state ---------- */
  var st = { open: false, tab: 'media', hint: null, hintTimer: 0, playing: false, track: 0, pos: 0,
             timer: null, game: null, quests: {}, done: false };
  var tracks = [
    ['Late Night Compile', 'The Demo Band', 192],
    ['Merge Conflict Blues', 'The Demo Band', 224],
    ['Green Build', 'The Demo Band', 171]
  ];
  var todos = [
    { t: 'Ship the thing', d: false },
    { t: 'Reply to mom', d: false },
    { t: 'Drink some water', d: true }
  ];
  var clips = [
    { t: 'https://nock.akhilvarute.me', p: true },
    { t: '#FF4D14', p: false, c: '#FF4D14' },
    { t: 'cargo build --release', p: false },
    { t: 'C:\\Users\\you\\Projects\\nock', p: false },
    { t: 'see you at standup', p: false }
  ];

  /* ---------- helpers ---------- */
  function pad(n) { return n < 10 ? '0' + n : '' + n; }
  function fmt(s) { return Math.floor(s / 60) + ':' + pad(Math.floor(s % 60)); }
  function clock() {
    var d = new Date(), h = d.getHours(), m = d.getMinutes();
    return { bar: pad(h) + ':' + pad(m), pill: (h % 12 || 12) + ':' + pad(m) + (h < 12 ? ' AM' : ' PM') };
  }
  var toastEl = $('#toast'), toastT = 0;
  function toast(msg) {
    toastEl.textContent = msg; toastEl.classList.add('on');
    clearTimeout(toastT); toastT = setTimeout(function () { toastEl.classList.remove('on'); }, 2200);
  }

  /* ---------- sizing ---------- */
  function size() {
    K = Math.min(1, (window.innerWidth - 16) / 600);
    notch.style.setProperty('--k', K);
    var w, h, rad;
    if (st.open) { w = 600 * K; h = 34 + 216 * K; rad = 26; }
    else if (st.hint) { w = Math.min(window.innerWidth - 16, st.hint.w); h = 34; rad = 16; }
    else if (st.timer) { w = 196; h = 34; rad = 15; }
    else { w = 176; h = 34; rad = 14; }
    notch.style.setProperty('--w', w + 'px');
    notch.style.setProperty('--h', h + 'px');
    notch.style.setProperty('--rad', rad + 'px');
  }
  window.addEventListener('resize', size);

  function renderPill() {
    var kind = '';
    if (st.hint) { pillText.textContent = st.hint.text; kind = st.hint.kind; }
    else if (st.timer) {
      pillText.textContent = fmt(st.timer.left);
      $('#pillProg').style.width = (100 - st.timer.left / st.timer.total * 100) + '%';
    } else pillText.textContent = clock().pill;
    pill.dataset.kind = kind;
    notch.classList.toggle('timing', !!st.timer && !st.hint);
    notch.classList.toggle('playing', st.playing);
    pill.setAttribute('aria-label', st.open ? 'Collapse the notch' : 'Open the notch');
  }

  /* ---------- quests ---------- */
  var cv = $('#confetti'), cx = cv.getContext('2d'), parts = [];
  function burst() {
    if (RM) return;
    cv.width = innerWidth; cv.height = innerHeight;
    var cols = ['#ff4d14', '#669eff', '#ffb020', '#3ddc84', '#ece7da'];
    for (var i = 0; i < 120; i++) parts.push({ x: innerWidth / 2, y: 60, vx: (Math.random() - .5) * 14, vy: Math.random() * 9 + 2,
      r: Math.random() * 6 + 3, c: cols[i % 5], a: 1, rot: Math.random() * 6 });
    (function step() {
      cx.clearRect(0, 0, cv.width, cv.height);
      parts = parts.filter(function (p) { return p.a > 0.02 && p.y < innerHeight + 20; });
      parts.forEach(function (p) {
        p.x += p.vx; p.y += p.vy; p.vy += .35; p.vx *= .99; p.a -= .006; p.rot += .2;
        cx.globalAlpha = p.a; cx.fillStyle = p.c; cx.save(); cx.translate(p.x, p.y); cx.rotate(p.rot); cx.fillRect(-p.r, -p.r / 2, p.r * 2, p.r); cx.restore();
      });
      if (parts.length) requestAnimationFrame(step); else cx.clearRect(0, 0, cv.width, cv.height);
    })();
  }
  function quest(id) {
    if (st.quests[id]) return;
    st.quests[id] = true;
    var el = $('[data-q="' + id + '"]'); if (el) el.classList.add('done');
    var n = Object.keys(st.quests).length;
    $('#qCount').textContent = n + ' / 5';
    if (n === 5 && !st.done) { st.done = true; $('#unlock').classList.add('on'); burst(); }
  }

  /* ---------- open / close / tabs ---------- */
  function setOpen(v) {
    if (st.open === v) return;
    st.open = v;
    notch.classList.toggle('open', v);
    pill.setAttribute('aria-expanded', v);
    panel.inert = !v;
    if (v) { document.body.classList.add('touched'); quest('open'); }
    else stopGame();
    size(); renderPill();
  }
  pill.addEventListener('click', function () { setOpen(!st.open); });
  pill.addEventListener('pointerenter', function () {
    if (st.open || RM) return;
    notch.classList.remove('jig'); void notch.offsetWidth; notch.classList.add('jig');
  });
  document.addEventListener('pointerdown', function (e) {
    if (st.open && !notch.contains(e.target) && !e.target.closest('[data-hint]')) setOpen(false);
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { if (st.game) stopGame(); else setOpen(false); }
    if (e.ctrlKey && e.altKey && (e.key === 't' || e.key === 'T')) { e.preventDefault(); quickAdd(); }
  });

  function showTab(name) {
    st.tab = name;
    $$('.tab').forEach(function (t) { t.setAttribute('aria-selected', t.dataset.tab === name); t.tabIndex = t.dataset.tab === name ? 0 : -1; });
    $$('.page').forEach(function (p) { p.hidden = p.dataset.page !== name; });
    if (name !== 'games') stopGame();
    if (name === 'stats') statsTick();
  }
  $$('.tab').forEach(function (t) {
    t.addEventListener('click', function () { showTab(t.dataset.tab); });
    t.addEventListener('keydown', function (e) {
      var tabs = $$('.tab'), i = tabs.indexOf(t);
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        var n = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
        n.focus(); showTab(n.dataset.tab); e.preventDefault();
      }
    });
  });
  $$('[data-toast]').forEach(function (b) { b.addEventListener('click', function () { toast(b.dataset.toast); }); });

  /* ---------- hints on the pill ---------- */
  function hint(text, kind, ms, w) {
    st.hint = { text: text, kind: kind, w: w || 250 };
    clearTimeout(st.hintTimer);
    size(); renderPill();
    st.hintTimer = setTimeout(function () { st.hint = null; size(); renderPill(); }, ms || 3200);
  }
  function quickAdd() {
    setOpen(true); showTab('today');
    setTimeout(function () { $('#addInput').focus(); }, 380);
  }
  $$('[data-hint]').forEach(function (b) {
    b.addEventListener('click', function () {
      var k = b.dataset.hint;
      if (st.open) setOpen(false);
      if (k === 'battery') hint('Battery low 14%, 22 min left', 'warn', 3600, 300);
      else if (k === 'meeting') hint('Standup in 5 min', 'warn', 3600, 230);
      else if (k === 'copy') { hint('Link copied', 'ok', 2800, 190); quest('copy'); }
      else if (k === 'quick') quickAdd();
    });
  });
  document.addEventListener('copy', function () {
    var s = (window.getSelection() || '').toString().trim(); if (!s) return;
    var label = /^https?:\/\//i.test(s) ? 'Link' : /^#[0-9a-f]{3,8}$/i.test(s) ? 'Colour' : /^[a-z]:\\|^\//i.test(s) ? 'Path' : s.length > 3 ? 'Code' : '';
    if (label) { hint(label + ' copied', 'ok', 2800, 200); if (label === 'Link') quest('copy'); }
  });

  /* ---------- media ---------- */
  function loadTrack() {
    var t = tracks[st.track]; st.pos = 0;
    $('#mTitle').textContent = t[0]; $('#mSub').textContent = t[1]; $('#mDur').textContent = fmt(t[2]);
    mediaView();
  }
  function mediaView() {
    var t = tracks[st.track], p = st.pos / t[2] * 100;
    $('#seekFill').style.width = p + '%'; $('#seek').style.setProperty('--p', p + '%'); $('#seek').setAttribute('aria-valuenow', Math.round(p));
    $('#mCur').textContent = fmt(st.pos);
    var b = $('#mPlay'); b.setAttribute('aria-label', st.playing ? 'Pause' : 'Play');
    b.innerHTML = '<svg class="icon"><use href="#i-' + (st.playing ? 'pause' : 'play') + '"/></svg>';
  }
  $('#mPlay').addEventListener('click', function () { st.playing = !st.playing; mediaView(); renderPill(); });
  $('#mNext').addEventListener('click', function () { st.track = (st.track + 1) % tracks.length; loadTrack(); });
  $('#mPrev').addEventListener('click', function () { st.track = (st.track + tracks.length - 1) % tracks.length; loadTrack(); });
  function seekTo(e) {
    var r = $('#seek').getBoundingClientRect(), f = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    st.pos = f * tracks[st.track][2]; mediaView();
  }
  var seeking = false;
  $('#seek').addEventListener('pointerdown', function (e) { seeking = true; this.setPointerCapture(e.pointerId); seekTo(e); });
  $('#seek').addEventListener('pointermove', function (e) { if (seeking) seekTo(e); });
  $('#seek').addEventListener('pointerup', function () { seeking = false; });
  $('#seek').addEventListener('keydown', function (e) {
    if (e.key === 'ArrowRight') st.pos = Math.min(tracks[st.track][2], st.pos + 5);
    else if (e.key === 'ArrowLeft') st.pos = Math.max(0, st.pos - 5); else return;
    mediaView(); e.preventDefault();
  });

  /* ---------- tasks and focus timer ---------- */
  function renderTodos() {
    var l = $('#tList'); l.innerHTML = '';
    todos.forEach(function (td, i) {
      var r = document.createElement('div'); r.className = 'task';
      r.innerHTML = '<input type="checkbox" id="td' + i + '"' + (td.d ? ' checked' : '') + '><label for="td' + i + '"></label>' +
        '<button class="btn-i" aria-label="Start a focus timer on this task"><svg class="icon"><use href="#i-play"/></svg></button>';
      $('label', r).textContent = td.t;
      $('input', r).addEventListener('change', function () { td.d = this.checked; });
      $('button', r).addEventListener('click', function () { startFocus(td.t); });
      l.appendChild(r);
    });
  }
  $('#addForm').addEventListener('submit', function (e) {
    e.preventDefault();
    var v = $('#addInput').value.trim(); if (!v) return;
    todos.unshift({ t: v, d: false }); $('#addInput').value = ''; renderTodos(); quest('task');
    toast('Saved');
  });
  function startFocus(name) {
    clearInterval(st.timer && st.timer.iv);
    st.timer = { left: 25 * 60, total: 25 * 60, name: name };
    st.timer.iv = setInterval(function () {
      st.timer.left--;
      if (st.timer.left <= 0) { clearInterval(st.timer.iv); st.timer = null; hint('Focus done', 'ok', 4000, 200); burst(); }
      size(); renderPill();
    }, 1000);
    quest('focus'); toast('Focus started: ' + name); setOpen(false);
  }

  /* ---------- stats ---------- */
  var cpu = 4, mem = 39, up = 134 * 60;
  function ring(id, v) { $(id).style.strokeDashoffset = 314 - 314 * v / 100; }
  function statsTick() {
    cpu = Math.max(2, Math.min(34, cpu + (Math.random() - .5) * 9)); mem = Math.max(34, Math.min(46, mem + (Math.random() - .5) * 2));
    $('#cpuV').textContent = Math.round(cpu) + '%'; $('#memV').textContent = Math.round(mem) + '%';
    ring('#ringCpu', cpu); ring('#ringMem', mem);
    $('#upV').textContent = Math.floor(up / 3600) + 'h ' + Math.floor(up % 3600 / 60) + 'm';
  }

  /* ---------- clipboard ---------- */
  function renderClips() {
    var q = $('#cSearch').value.toLowerCase(), l = $('#cList'); l.innerHTML = '';
    clips.filter(function (c) { return c.t.toLowerCase().indexOf(q) > -1; })
      .sort(function (a, b) { return (b.p ? 1 : 0) - (a.p ? 1 : 0); })
      .forEach(function (c) {
        var r = document.createElement('div'); r.className = 'clip';
        r.innerHTML = (c.c ? '<i class="sw" style="background:' + c.c + '"></i>' : '') + '<button class="txt"></button><button class="pin" aria-label="Pin"><svg class="icon" style="width:16px;height:16px"><use href="#i-pin"/></svg></button>';
        $('.txt', r).textContent = c.t; $('.pin', r).setAttribute('aria-pressed', !!c.p);
        $('.txt', r).addEventListener('click', function () { toast('Pasted: ' + c.t); });
        $('.pin', r).addEventListener('click', function () { c.p = !c.p; renderClips(); });
        l.appendChild(r);
      });
  }
  $('#cSearch').addEventListener('input', renderClips);

  /* ---------- Brick Breaker (the one game playable here) ---------- */
  var cvs = $('#gCv'), g = cvs.getContext('2d'), raf = 0, G = null;
  function stopGame() {
    cancelAnimationFrame(raf); st.game = null; G = null;
    $('#gBox').classList.remove('on'); $('#gMenu').style.display = '';
  }
  function startGame() {
    st.game = 'brick'; $('#gMenu').style.display = 'none'; $('#gBox').classList.add('on');
    var W = 568, H = 156;
    G = { px: W / 2, bx: W / 2, by: H - 22, vx: 2.6, vy: -2.6, score: 0, lives: 3, run: false, bricks: [] };
    for (var r = 0; r < 4; r++) for (var c = 0; c < 12; c++) G.bricks.push({ x: 10 + c * 46, y: 22 + r * 14, w: 42, h: 10, on: true, col: ['#ff4d14', '#ffb020', '#3ddc84', '#669eff'][r] });
    function loop() {
      if (!G) return;
      g.clearRect(0, 0, W, H);
      G.bricks.forEach(function (b) { if (b.on) { g.fillStyle = b.col; g.beginPath(); g.roundRect(b.x, b.y, b.w, b.h, 3); g.fill(); } });
      g.fillStyle = '#fff'; g.beginPath(); g.roundRect(G.px - 34, H - 12, 68, 6, 3); g.fill();
      g.beginPath(); g.arc(G.bx, G.by, 5, 0, 6.3); g.fill();
      if (G.run) {
        G.bx += G.vx; G.by += G.vy;
        if (G.bx < 5 || G.bx > W - 5) G.vx *= -1;
        if (G.by < 5) G.vy *= -1;
        if (G.by > H - 17 && G.by < H - 8 && Math.abs(G.bx - G.px) < 40 && G.vy > 0) { G.vy *= -1; G.vx = (G.bx - G.px) / 12; }
        G.bricks.forEach(function (b) {
          if (b.on && G.bx > b.x && G.bx < b.x + b.w && G.by > b.y && G.by < b.y + b.h) { b.on = false; G.vy *= -1; G.score++; quest('brick'); }
        });
        if (G.by > H) { G.lives--; G.run = false; G.bx = G.px; G.by = H - 22; if (G.lives < 0) { G.score = 0; G.lives = 3; G.bricks.forEach(function (b) { b.on = true; }); } }
        if (!G.bricks.some(function (b) { return b.on; })) { G.bricks.forEach(function (b) { b.on = true; }); G.run = false; }
      } else { G.bx = G.px; g.fillStyle = '#8a8a8a'; g.font = '500 12px monospace'; g.textAlign = 'center'; g.fillText('click to launch', W / 2, 110); }
      $('#gHud').textContent = 'score ' + G.score + '   lives ' + Math.max(G.lives, 0);
      raf = requestAnimationFrame(loop);
    }
    loop();
  }
  cvs.addEventListener('pointermove', function (e) { if (!G) return; var r = cvs.getBoundingClientRect(); G.px = Math.max(34, Math.min(534, (e.clientX - r.left) / r.width * 568)); });
  cvs.addEventListener('pointerdown', function () { if (G) G.run = true; });
  $('#gBack').addEventListener('click', stopGame);
  $('[data-game="brick"]').addEventListener('click', startGame);
  if (typeof CanvasRenderingContext2D.prototype.roundRect !== 'function') CanvasRenderingContext2D.prototype.roundRect = function (x, y, w, h) { this.rect(x, y, w, h); };

  /* ---------- clocks, dates, 1 Hz tick ---------- */
  var d0 = new Date();
  $('#tDay').textContent = d0.toLocaleDateString('en', { weekday: 'short' }) + ' ' + d0.getDate();
  $('#tMonth').textContent = d0.toLocaleDateString('en', { month: 'long' });
  setInterval(function () {
    $('#barClock').textContent = clock().bar;
    if (st.playing) { st.pos++; if (st.pos >= tracks[st.track][2]) { st.track = (st.track + 1) % tracks.length; loadTrack(); } mediaView(); }
    up++; if (st.open && st.tab === 'stats') statsTick();
    if (!st.hint && !st.timer) renderPill();
  }, 1000);
  $('#barClock').textContent = clock().bar;

  /* ---------- scroll reveals, counters, bars ---------- */
  var io = 'IntersectionObserver' in window ? new IntersectionObserver(function (es) {
    es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } });
  }, { threshold: .15 }) : null;
  $$('.reveal').forEach(function (el) { io ? io.observe(el) : el.classList.add('in'); });

  function countUp(el) {
    var to = +el.dataset.count, suf = el.dataset.suffix || '', fixed = el.dataset.fixed;
    if (RM || to === 0) { el.textContent = (fixed ? '0' : to) + suf; return; }
    var t0 = performance.now();
    (function f(t) {
      var p = Math.min(1, (t - t0) / 1200), v = Math.round(to * (1 - Math.pow(1 - p, 3)));
      el.textContent = v + suf; if (p < 1) requestAnimationFrame(f);
    })(t0);
  }
  var io2 = io && new IntersectionObserver(function (es) {
    es.forEach(function (e) { if (e.isIntersecting) { countUp(e.target); io2.unobserve(e.target); } });
  }, { threshold: .6 });
  $$('[data-count]').forEach(function (el) { io2 ? io2.observe(el) : 0; });

  /* ---------- latest release: direct download links and version ---------- */
  function hrefs(rel) {
    var setup = '', portable = '';
    (rel.assets || []).forEach(function (a) {
      if (/^NockSetup-.*\.exe$/i.test(a.name)) setup = a.browser_download_url;
      if (/^Nock-portable-.*\.exe$/i.test(a.name)) portable = a.browser_download_url;
    });
    $$('[data-dl="setup"]').forEach(function (a) { if (setup) a.href = setup; });
    $$('[data-dl="portable"]').forEach(function (a) { if (portable) a.href = portable; });
    var v = rel.tag_name || '';
    if (v) {
      $('#verLine').textContent = 'Latest: ' + v + '. Free, MIT licensed. Installer or one portable .exe.';
      $('#verLine2').textContent = 'Latest: ' + v + '. Free. Windows 10/11. The installer adds a Start menu entry; the portable file is just the app.';
    }
  }
  try {
    fetch('https://api.github.com/repos/Akhilesh-Varute/Nock/releases/latest').then(function (r) { return r.ok ? r.json() : null; })
      .then(function (j) { if (j) hrefs(j); }).catch(function () {});
  } catch (e) {}

  if (!/Windows/i.test(navigator.userAgent)) $('#osNote').style.display = 'block';

  /* ---------- init ---------- */
  renderTodos(); renderClips(); loadTrack(); statsTick(); showTab('media'); size(); renderPill();
})();
