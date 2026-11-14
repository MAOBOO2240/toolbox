// ==UserScript==
// @name         X 診斷（工具箱）
// @namespace    maoboo-diag
// @version      2
// @description  查 x.com 為什麼停在標誌：記下網頁在等哪些東西，20 秒後把報告存到「下載」。只記技術資訊，不記帳號密碼、Cookie 內容、貼文內容。查完請刪除。
// @match        https://x.com/*
// @match        https://twitter.com/*
// @run-at       document-start
// @noframes
// @grant        none
// ==/UserScript==
(function () {
  // 只在最上層的網頁跑；過了期限就什麼都不做（忘記刪也沒關係）
  if (window.top !== window || window.__mbUser) return;
  if (Date.now() > 1795276800000) return;
  window.__mbUser = true;
  window.__mbRaw = true;   // 不改網頁的 Worker
  window.__mbLite = true;  // 不換掉 Blob
  var VER = 'v2';
  var T0 = performance.now();
  var now = function () { return Math.round(performance.now() - T0); };
  var short = function (u) { try { return String(u).replace(/^https?:\/\//, '').split('?')[0].split('#')[0].slice(0, 96); } catch (_) { return '?'; } };

  /* ---------- 黑盒子：每做一件事就記在這個網站的儲存空間，網頁整個卡死的話，下次開啟時看得到上次停在哪 ---------- */
  var KEY = '__mbdiag_bb';
  var LSset = Storage.prototype.setItem, LSget = Storage.prototype.getItem, LSrem = Storage.prototype.removeItem;
  var lsSet = function (k, v) { try { LSset.call(localStorage, k, v); } catch (_) {} };
  var lsState = 'ok', prev = null;
  try {
    var rawPrev = LSget.call(localStorage, KEY);
    if (rawPrev) prev = JSON.parse(rawPrev);
  } catch (e) { lsState = String(e).slice(0, 80); }
  if (prev && (prev.done || !prev.at || Date.now() - prev.at > 30 * 60000)) prev = null;
  var CUR = '__mbdiag_cur';
  var prevCur = null;
  try { prevCur = LSget.call(localStorage, CUR); } catch (_) {}
  if (prev) prev.cur = prevCur;
  var bb = { id: Date.now().toString(36), at: Date.now(), href: short(location.href), hb: 0, marks: [], tl: [], counts: {}, done: false };
  var bbSave = function () { try { bb.vis = document.visibilityState + (document.hasFocus() ? ' 有焦點' : ' 沒焦點'); } catch (_) {} lsSet(KEY, JSON.stringify(bb)); };
  var pushMark = function (text) {
    bb.marks.push(text);
    if (bb.marks.length > 40) bb.marks.shift();
    bb.hb = now();
    bbSave();
  };
  window.__mbOnRec = function (e) { pushMark(e.n + ' [' + e.t + ']'); };
  bbSave();
  lsSet(CUR, '');

  /* ---------- 會讓網頁停下來等的功能：每次呼叫前先記「現在正要做這件事」，卡死的話下次就知道是哪一件 ---------- */
  var counts = bb.counts;
  var mark = function (name) {
    var c = counts[name] = (counts[name] || 0) + 1;
    lsSet(CUR, bb.id + '|' + name + '#' + c + '|' + now());
    if (c <= 2) pushMark(name + (c > 1 ? '#' + c : '') + ' [' + now() + ']');
  };
  var syncWrap = function (obj, keys, label) {
    if (!obj) return;
    keys.forEach(function (key) {
      try {
        var d = Object.getOwnPropertyDescriptor(obj, key);
        if (!d || typeof d.value !== 'function') return;
        var f = d.value, name = label + '.' + key;
        obj[key] = function () { mark(name); return f.apply(this, arguments); };
      } catch (_) {}
    });
  };
  var getWrap = function (obj, key, name) {
    try {
      var d = Object.getOwnPropertyDescriptor(obj, key);
      if (!d || !d.get || !d.configurable) return;
      var g = d.get;
      Object.defineProperty(obj, key, { configurable: true, enumerable: d.enumerable, get: function () { mark(name); return g.call(this); }, set: d.set });
    } catch (_) {}
  };
  var GLK = ['getParameter', 'getExtension', 'getSupportedExtensions', 'getShaderPrecisionFormat', 'getContextAttributes', 'readPixels', 'compileShader', 'linkProgram', 'drawArrays', 'drawElements', 'bufferData', 'texImage2D', 'finish', 'flush', 'getError'];
  if (window.WebGLRenderingContext) syncWrap(WebGLRenderingContext.prototype, GLK, 'webgl');
  if (window.WebGL2RenderingContext) syncWrap(WebGL2RenderingContext.prototype, GLK, 'webgl2');
  if (window.CanvasRenderingContext2D) syncWrap(CanvasRenderingContext2D.prototype, ['fillText', 'strokeText', 'measureText', 'getImageData', 'drawImage', 'isPointInPath'], 'canvas2d');
  if (window.OffscreenCanvasRenderingContext2D) syncWrap(OffscreenCanvasRenderingContext2D.prototype, ['fillText', 'measureText', 'getImageData'], 'offscreen2d');
  syncWrap(HTMLCanvasElement.prototype, ['toDataURL', 'toBlob', 'transferControlToOffscreen', 'captureStream'], 'canvas');
  if (window.OffscreenCanvas) syncWrap(OffscreenCanvas.prototype, ['transferToImageBitmap', 'convertToBlob'], 'offscreen');
  syncWrap(window, ['alert', 'confirm', 'prompt', 'print', 'open', 'openDatabase', 'showOpenFilePicker'], 'window');
  syncWrap(Storage.prototype, ['getItem', 'setItem', 'removeItem', 'key'], 'storage');
  syncWrap(History.prototype, ['pushState', 'replaceState', 'go', 'back'], 'history');
  syncWrap(Document.prototype, ['write', 'open', 'execCommand'], 'document');
  if (window.speechSynthesis) syncWrap(Object.getPrototypeOf(speechSynthesis), ['getVoices'], 'speech');
  syncWrap(Navigator.prototype, ['getGamepads', 'sendBeacon', 'requestMediaKeySystemAccess', 'requestMIDIAccess', 'vibrate', 'javaEnabled'], 'navigator');
  if (document.fonts) syncWrap(Object.getPrototypeOf(document.fonts), ['check'], 'fonts');
  if (navigator.mediaDevices) syncWrap(Object.getPrototypeOf(navigator.mediaDevices), ['getUserMedia', 'getDisplayMedia'], 'mediaDevices');
  ['hid', 'usb', 'serial', 'bluetooth'].forEach(function (k) { try { if (navigator[k]) syncWrap(Object.getPrototypeOf(navigator[k]), ['getDevices', 'getPorts', 'getAvailability'], k); } catch (_) {} });
  try { if (navigator.xr) syncWrap(Object.getPrototypeOf(navigator.xr), ['isSessionSupported', 'requestSession'], 'xr'); } catch (_) {}
  if (window.MediaSource) syncWrap(MediaSource, ['isTypeSupported'], 'MediaSource');
  syncWrap(HTMLMediaElement.prototype, ['canPlayType'], 'media');
  getWrap(Document.prototype, 'cookie', 'document.cookie');
  getWrap(Navigator.prototype, 'plugins', 'navigator.plugins');
  getWrap(Navigator.prototype, 'mimeTypes', 'navigator.mimeTypes');
  try { if (window.Notification) getWrap(Notification, 'permission', 'Notification.permission'); } catch (_) {}
  ['RTCPeerConnection', 'AudioContext', 'OfflineAudioContext', 'SharedWorker', 'WebSocket', 'EventSource', 'BroadcastChannel', 'FontFace', 'Notification', 'PaymentRequest'].forEach(function (k) {
    try {
      var O = window[k];
      if (typeof O !== 'function') return;
      var N = function () { mark('new ' + k); return Reflect.construct(O, arguments, new.target || N); };
      N.prototype = O.prototype;
      Object.getOwnPropertyNames(O).forEach(function (n) { if (!(n in N)) { try { Object.defineProperty(N, n, Object.getOwnPropertyDescriptor(O, n)); } catch (_) {} } });
      window[k] = N;
    } catch (_) {}
  });
  try { if (window.WebAssembly) { var WM = WebAssembly.Module; WebAssembly.Module = function (b) { mark('new WebAssembly.Module'); return Reflect.construct(WM, arguments, new.target || WebAssembly.Module); }; WebAssembly.Module.prototype = WM.prototype; ['exports', 'imports', 'customSections'].forEach(function (n) { WebAssembly.Module[n] = WM[n]; }); } } catch (_) {}
  try { var xo0 = XMLHttpRequest.prototype.open; XMLHttpRequest.prototype.open = function (m, u, a) { if (a === false) mark('同步的 XMLHttpRequest ' + short(u).slice(0, 50)); return xo0.apply(this, arguments); }; } catch (_) {}

  /* ---------- 看門狗：另一條執行緒每秒記一次「網頁主要的那條執行緒最後一次有反應是什麼時候」 ---------- */
  var wdPrev = null, wd = null;
  try {
    var rq = indexedDB.open('__mbdiag_wd', 1);
    rq.onupgradeneeded = function () { try { rq.result.createObjectStore('kv'); } catch (_) {} };
    rq.onsuccess = function () {
      try {
        if (!prev) { rq.result.close(); return; }
        var g = rq.result.transaction('kv', 'readonly').objectStore('kv').get('wd:' + prev.id);
        g.onsuccess = function () { wdPrev = g.result || null; try { rq.result.close(); } catch (_) {} };
        g.onerror = function () { try { rq.result.close(); } catch (_) {} };
      } catch (_) {}
    };
  } catch (_) {}
  try {
    var wsrc = "var last=0,run='',db=null,t0=Date.now();onmessage=function(e){if(e.data&&e.data.run)run=e.data.run;last=Date.now();};" +
      "var rq=indexedDB.open('__mbdiag_wd',1);rq.onupgradeneeded=function(){try{rq.result.createObjectStore('kv');}catch(e){}};rq.onsuccess=function(){db=rq.result;};" +
      "setInterval(function(){if(Date.now()-t0>120000){try{db&&db.close();}catch(e){}close();return;}if(!db||!run)return;try{db.transaction('kv','readwrite').objectStore('kv').put({run:run,mainLast:last,workerNow:Date.now()},'wd:'+run);}catch(e){}},1000);";
    wd = new Worker(URL.createObjectURL(new Blob([wsrc], { type: 'text/javascript' })));
    wd.postMessage({ run: bb.id });
    var wdTimer = setInterval(function () { try { wd.postMessage(1); } catch (_) {} }, 250);
    setTimeout(function () {
      try { clearInterval(wdTimer); wd.terminate(); } catch (_) {}
      // 這一次有跑完：留下來的小記錄收乾淨
      try { if (bb.done) { LSrem.call(localStorage, CUR); indexedDB.deleteDatabase('__mbdiag_wd'); } } catch (_) {}
    }, 60000);
  } catch (_) {}

  var host = '不是在工具箱的瀏覽器裡';
  try { if (typeof GM_info !== 'undefined' && GM_info) host = String(GM_info.scriptHandler) + ' ' + String(GM_info.version || ''); } catch (_) {}
  var uaFull = {};
  try {
    if (navigator.userAgentData) navigator.userAgentData.getHighEntropyValues(['uaFullVersion', 'fullVersionList', 'platformVersion', 'architecture']).then(function (v) {
      uaFull = { 完整版本: v.uaFullVersion, 清單: (v.fullVersionList || []).map(function (b) { return b.brand + ' ' + b.version; }).join(', '), Windows: v.platformVersion, 架構: v.architecture };
    }, function () {});
  } catch (_) {}

  /* ---------- 顯示卡的名稱（網頁自己問的時候順便記下來，不另外問） ---------- */
  var gfx = {};
  var hookGL = function (C) {
    try {
      if (!C) return;
      var gp = C.prototype.getParameter;
      C.prototype.getParameter = function (k) {
        var v = gp.apply(this, arguments);
        try { if (k === 0x9245) gfx.vendor = String(v).slice(0, 60); else if (k === 0x9246) gfx.renderer = String(v).slice(0, 120); } catch (_) {}
        return v;
      };
    } catch (_) {}
  };
  hookGL(window.WebGLRenderingContext);
  hookGL(window.WebGL2RenderingContext);
  try {
    if (navigator.gpu) {
      var gproto = Object.getPrototypeOf(navigator.gpu), ra = gproto.requestAdapter;
      gproto.requestAdapter = function () {
        var p = ra.apply(this, arguments);
        try {
          p.then(function (a) {
            if (!a) { gfx.webgpu = '沒有'; return; }
            var i = a.info || {};
            gfx.webgpu = [i.vendor, i.architecture, i.device, i.description].filter(Boolean).join(' / ').slice(0, 120) || '有';
            if (i.isFallbackAdapter || a.isFallbackAdapter) gfx.webgpu += '（軟體）';
          }, function (x) { gfx.webgpu = '失敗 ' + String(x).slice(0, 40); });
        } catch (_) {}
        return p;
      };
    } else gfx.webgpu = '網頁拿不到';
  } catch (_) {}

  /* ---------- 檔案載入、錯誤、被擋的東西 ---------- */
  var loads = [], csp = [], vis = [];
  var onRes = function (kind) {
    return function (ev) {
      try {
        var t = ev.target;
        if (!t || !t.tagName || loads.length >= 120) return;
        var tag = t.tagName;
        if (tag !== 'SCRIPT' && tag !== 'LINK' && tag !== 'IFRAME' && tag !== 'IMG') return;
        var u = t.src || t.href || '';
        if (!u || (tag === 'IMG' && kind === 'ok')) return;
        loads.push(now() + ' ' + kind + ' ' + tag.toLowerCase() + ' ' + short(u).slice(-70));
      } catch (_) {}
    };
  };
  document.addEventListener('load', onRes('ok'), true);
  document.addEventListener('error', onRes('失敗'), true);
  document.addEventListener('securitypolicyviolation', function (ev) {
    if (csp.length < 12) csp.push(now() + ' ' + ev.violatedDirective + ' ' + short(ev.blockedURI).slice(0, 60));
  });
  var visRec = function (what) { if (vis.length < 20) vis.push(now() + ' ' + what); };
  visRec('一開始 ' + document.visibilityState + (document.hasFocus() ? ' 有焦點' : ' 沒焦點'));
  document.addEventListener('visibilitychange', function () { visRec(document.visibilityState); });
  window.addEventListener('focus', function () { visRec('focus'); });
  window.addEventListener('blur', function () { visRec('blur'); });
  window.addEventListener('pagehide', function () { visRec('pagehide'); bbSave(); });
  window.addEventListener('online', function () { visRec('online'); });
  window.addEventListener('offline', function () { visRec('offline'); });

  /* ---------- 網頁有沒有在動：心跳、卡頓 ---------- */
  var beats = 0, lastBeat = now(), maxGap = 0, maxGapAt = 0;
  setInterval(function () {
    var t = now(), g = t - lastBeat;
    if (g > maxGap) { maxGap = g; maxGapAt = t; }
    lastBeat = t;
    beats++;
    if (beats % 4 === 0 && !bb.done) { bb.hb = t; bbSave(); }
  }, 250);
  var longs = [];
  try {
    new PerformanceObserver(function (list) {
      list.getEntries().forEach(function (e) { if (longs.length < 40) longs.push(Math.round(e.startTime) + '+' + Math.round(e.duration)); });
    }).observe({ type: 'longtask', buffered: true });
  } catch (_) {}

  /* ---------- 畫面每隔一陣子的樣子 ---------- */
  var snaps = [];
  var snap = function () {
    try {
      var b = document.body;
      var one = {
        t: now(), ready: document.readyState, els: document.getElementsByTagName('*').length,
        txt: b ? (b.innerText || '').replace(/\s+/g, ' ').length : -1, kids: b ? b.children.length : -1,
        h1: !!document.querySelector('h1'), form: !!document.querySelector('form, input'), art: document.getElementsByTagName('article').length,
        scripts: document.scripts.length,
      };
      snaps.push(one);
      if (!bb.done && bb.tl.length < 12) { bb.tl.push(one.t + ':' + one.ready.slice(0, 1) + ' 元素' + one.els + ' 字' + one.txt + (one.h1 ? ' 有標題' : '') + (one.form ? ' 有表單' : '')); bbSave(); }
    } catch (e) { snaps.push({ t: now(), err: String(e).slice(0, 60) }); }
  };
  [100, 300, 600, 1000, 1500, 2000, 3500, 6000, 10000, 15000, 19500, 40000].forEach(function (ms) { setTimeout(snap, ms); });

  /* ---------- 自己做的小測試（8 秒時開始，每項最多等 5 秒） ---------- */
  var tests = {};
  var timed = function (name, fn) {
    var t = now();
    var p;
    try { p = Promise.resolve(fn()); } catch (e) { tests[name] = '丟出錯誤 ' + String(e).slice(0, 70); return Promise.resolve(); }
    tests[name] = '還在等';
    var done = false;
    return Promise.race([
      p.then(function (v) { done = true; tests[name] = String(v).slice(0, 90) + '（' + (now() - t) + ' 毫秒）'; }, function (e) { done = true; tests[name] = '失敗 ' + String(e).slice(0, 80) + '（' + (now() - t) + ' 毫秒）'; }),
      new Promise(function (r) { setTimeout(function () { if (!done) tests[name] = '等了 5 秒沒有回應'; r(); }, 5000); }),
    ]);
  };
  var runSelf = function () {
    var F = window.fetch.bind(window);
    timed('連線 x.com', function () { return F('/robots.txt', { cache: 'no-store' }).then(function (r) { return r.status; }); });
    timed('連線 abs.twimg.com', function () { return F('https://abs.twimg.com/favicons/twitter.3.ico', { mode: 'no-cors', cache: 'no-store' }).then(function (r) { return r.type; }); });
    timed('連線 api.x.com', function () { return F('https://api.x.com/1.1/hashflags.json', { credentials: 'include', cache: 'no-store' }).then(function (r) { return r.status; }); });
    timed('連線 jf.x.com', function () { return F('https://jf.x.com/', { mode: 'no-cors', cache: 'no-store' }).then(function (r) { return r.type; }); });
    timed('Worker', function () {
      return new Promise(function (res, rej) {
        var w = new Worker(URL.createObjectURL(new Blob(['postMessage(1)'], { type: 'text/javascript' })));
        w.onmessage = function () { w.terminate(); res('ok'); };
        w.onerror = function (e) { rej(e.message || 'error'); };
      });
    });
    timed('IndexedDB', function () {
      return new Promise(function (res, rej) {
        var r = indexedDB.open('__mbdiag', 1);
        r.onsuccess = function () { try { r.result.close(); indexedDB.deleteDatabase('__mbdiag'); } catch (_) {} res('ok'); };
        r.onerror = function () { rej(r.error); };
        r.onblocked = function () { rej('blocked'); };
      });
    });
    timed('儲存空間', function () { return navigator.storage.estimate().then(function (e) { return Math.round((e.usage || 0) / 1048576) + ' / ' + Math.round((e.quota || 0) / 1048576) + ' MB'; }); });
    timed('快取', function () { return caches.keys().then(function (k) { return k.length + ' 個'; }); });
    timed('鎖', function () { return navigator.locks.query().then(function (q) { return (q.held || []).length + ' 個占用、' + (q.pending || []).length + ' 個在等'; }); });
    timed('背景程式', function () {
      return navigator.serviceWorker.getRegistrations().then(function (l) {
        return l.length + ' 個' + (navigator.serviceWorker.controller ? '、這一頁由它接管' : '') + l.map(function (r) { return ' ' + short(r.scope) + ':' + ((r.active && r.active.state) || (r.installing && 'installing') || (r.waiting && 'waiting') || '?'); }).join('');
      });
    });
    timed('畫面更新', function () { return new Promise(function (res) { requestAnimationFrame(function () { requestAnimationFrame(function () { res('ok'); }); }); }); });
    timed('空閒時執行', function () { return new Promise(function (res) { if (!window.requestIdleCallback) return res('沒有這個功能'); requestIdleCallback(function () { res('ok'); }, { timeout: 4000 }); }); });
    timed('訊息通道', function () { return new Promise(function (res) { var c = new MessageChannel(); c.port1.onmessage = function () { res('ok'); }; c.port2.postMessage(1); }); });
    timed('排程', function () { return window.scheduler && scheduler.postTask ? scheduler.postTask(function () { return 'ok'; }) : '沒有這個功能'; });
    timed('計時器', function () { return new Promise(function (res) { var t = performance.now(); setTimeout(function () { res('慢了 ' + Math.round(performance.now() - t - 50) + ' 毫秒'); }, 50); }); });
    timed('字型', function () { return document.fonts.ready.then(function () { return document.fonts.status + ' ' + document.fonts.size + ' 個'; }); });
    try {
      document.cookie = '__mbdiag=1; path=/; max-age=30; SameSite=Lax; Secure';
      tests['Cookie'] = /(^|; )__mbdiag=1/.test(document.cookie) ? '寫得進去' : '寫不進去';
      document.cookie = '__mbdiag=; path=/; max-age=0; Secure';
    } catch (e) { tests['Cookie'] = '丟出錯誤 ' + String(e).slice(0, 60); }
    try { sessionStorage.setItem('__mbdiag', '1'); tests['sessionStorage'] = sessionStorage.getItem('__mbdiag') === '1' ? 'ok' : '讀不回來'; sessionStorage.removeItem('__mbdiag'); } catch (e) { tests['sessionStorage'] = '丟出錯誤 ' + String(e).slice(0, 60); }
  };
  setTimeout(function () { try { runSelf(); } catch (e) { tests['小測試'] = '壞掉 ' + String(e).slice(0, 80); } }, 8000);

  /* ---------- 報告 ---------- */
  var prevInfo = function () {
    if (!prev) return null;
    var o = { 幾秒前開始: Math.round((Date.now() - prev.at) / 1000), 網址: prev.href, 最後有反應的毫秒: prev.hb, 最後正在做: prev.cur, 最後做的事: prev.marks, 畫面變化: prev.tl, 呼叫次數: prev.counts };
    if (wdPrev) o.看門狗 = { 主執行緒最後有反應的毫秒: wdPrev.mainLast - prev.at, 另一條執行緒活到毫秒: wdPrev.workerNow - prev.at };
    return o;
  };
  var build = function (why) {
    var out = { 報告: 'MAOBOO X 診斷 ' + VER, 原因: why, 時間: new Date().toISOString(), 經過毫秒: now(), 網址: short(location.href) };
    var put = function (k, fn) { try { out[k] = fn(); } catch (e) { out[k] = '取不到：' + String(e).slice(0, 80); } };
    put('執行的地方', function () { return host; });
    put('瀏覽器', function () { return { UA: navigator.userAgent, 版本: uaFull, WebView2: !!(window.chrome && window.chrome.webview) }; });
    put('上次沒跑完', function () { return prevInfo(); });
    put('呼叫次數', function () { return counts; });
    put('儲存空間能不能用', function () { return lsState; });
    put('心跳', function () { return { 次數: beats, 應該大約: Math.round(now() / 250), 最長停頓毫秒: maxGap, 發生在: maxGapAt, 長時間卡住的工作: longs }; });
    put('看得見與焦點', function () { return vis.concat(['現在 ' + document.visibilityState + (document.hasFocus() ? ' 有焦點' : ' 沒焦點')]); });
    put('畫面', function () { return snaps; });
    put('網路', function () {
      var c = navigator.connection || {};
      return { 連線中: navigator.onLine, 類型: c.effectiveType, 往返毫秒: c.rtt, 下載Mbps: c.downlink, 省流量: c.saveData };
    });
    put('顯示卡', function () { return gfx; });
    put('環境', function () {
      var mm = function (q) { return matchMedia(q).matches; };
      return {
        時區: Intl.DateTimeFormat().resolvedOptions().timeZone, 語言: navigator.languages.join(','), 平台: navigator.platform,
        螢幕: screen.width + 'x' + screen.height + ' 色深' + screen.colorDepth, 視窗: innerWidth + 'x' + innerHeight + ' 外框' + outerWidth + 'x' + outerHeight, 縮放: devicePixelRatio,
        深色: mm('(prefers-color-scheme: dark)'), 減少動態: mm('(prefers-reduced-motion: reduce)'), HDR: mm('(dynamic-range: high)'), 廣色域: mm('(color-gamut: p3)'),
        觸控: navigator.maxTouchPoints, Cookie開著: navigator.cookieEnabled, 跨來源隔離: !!window.crossOriginIsolated,
        全域: Object.keys(window).filter(function (k) { return /TAURI|^ipc$|^__mb|^GM_|tampermonkey|userscript/i.test(k); }).join(','),
      };
    });
    put('網頁結構', function () {
      var d = document.documentElement, b = document.body;
      var desc = function (el) { return el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.className && typeof el.className === 'string' ? '.' + el.className.replace(/\s+/g, '.').slice(0, 50) : ''); };
      var attrs = {};
      Array.prototype.forEach.call(d.attributes, function (a) { attrs[a.name] = String(a.value).slice(0, 40); });
      return {
        html屬性: attrs, 標題: document.title.slice(0, 60),
        html底下: Array.prototype.map.call(d.children, desc).slice(0, 10),
        body底下: b ? Array.prototype.map.call(b.children, desc).slice(0, 14) : null,
        等待中的標誌: !!document.querySelector('[class*="r-6uq4to"], #placeholder'),
        行內樣式數: document.querySelectorAll('style').length, 內嵌框: Array.prototype.map.call(document.querySelectorAll('iframe'), function (f) { return short(f.src).slice(0, 60); }).slice(0, 6),
      };
    });
    put('載入的檔案', function () {
      var es = performance.getEntriesByType('resource');
      var names = {};
      es.forEach(function (e) { names[e.name.split('#')[0]] = 1; });
      var pending = [];
      Array.prototype.forEach.call(document.querySelectorAll('script[src], link[href], iframe[src]'), function (el) {
        var rel = (el.getAttribute('rel') || '').toLowerCase();
        if (el.tagName === 'LINK' && !/stylesheet|modulepreload|preload/.test(rel)) return;
        var u = (el.src || el.href || '').split('#')[0];
        if (u && !/^(data|blob|about):/.test(u) && !names[u] && pending.length < 40) pending.push(el.tagName.toLowerCase() + (rel ? '[' + rel + ']' : '') + ' ' + short(u).slice(-80));
      });
      var nav = performance.getEntriesByType('navigation')[0] || {};
      return {
        主網頁: { 種類: nav.type, 轉址次數: nav.redirectCount, 協定: nav.nextHopProtocol, 收到第一個位元組: Math.round(nav.responseStart || 0), 收完: Math.round(nav.responseEnd || 0), DOM好了: Math.round(nav.domContentLoadedEventEnd || 0), 載入完成: Math.round(nav.loadEventEnd || 0), 大小: nav.encodedBodySize },
        總數: es.length,
        清單: es.slice(0, 110).map(function (e) { return Math.round(e.startTime) + '-' + Math.round(e.responseEnd) + ' ' + e.initiatorType + ' ' + (e.nextHopProtocol || '-') + ' ' + short(e.name).slice(-72); }),
        網頁裡有但還沒載完或失敗: pending,
        載入事件: loads,
      };
    });
    put('被安全規則擋掉', function () { return csp; });
    put('Cookie名稱', function () { return document.cookie.split(';').map(function (c) { return c.split('=')[0].trim(); }).filter(Boolean).slice(0, 40).join(','); });
    put('儲存空間的項目', function () {
      var ks = [];
      for (var i = 0; i < localStorage.length && ks.length < 40; i++) { var k = localStorage.key(i); if (k !== KEY && k !== CUR) ks.push(String(k).slice(0, 48)); }
      return { 數量: localStorage.length, 名稱: ks };
    });
    put('小測試', function () { return tests; });
    put('網頁做了什麼', function () {
      var s = JSON.parse(window.__mbSummary());
      delete s.text;
      delete s.href;
      return s;
    });
    return out;
  };

  /* ---------- 右下角的小視窗 ---------- */
  var panel = null, statusEl = null, detailEl = null, latest = '';
  var mk = function (tag, css, text) { var el = document.createElement(tag); el.style.cssText = css; if (text) el.textContent = text; return el; };
  var fileName = function (tag) { var d = new Date(); var p = function (n) { return (n < 10 ? '0' : '') + n; }; return 'x-diag-report-' + p(d.getHours()) + p(d.getMinutes()) + p(d.getSeconds()) + (tag ? '-' + tag : '') + '.txt'; };
  var download = function (text, tag) {
    try {
      var a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }));
      var name = fileName(tag);
      a.download = name;
      a.style.display = 'none';
      (document.body || document.documentElement).appendChild(a);
      a.click();
      setTimeout(function () { try { a.remove(); } catch (_) {} }, 2000);
      return name;
    } catch (_) { return ''; }
  };
  var copy = function (text, btn) {
    var ok = function () { btn.textContent = '已複製，貼給 Claude'; };
    var fallback = function () {
      try {
        var ta = mk('textarea', 'position:fixed;left:0;top:0;width:10px;height:10px;opacity:.01');
        ta.value = text;
        (document.body || document.documentElement).appendChild(ta);
        ta.focus(); ta.select();
        var done = document.execCommand('copy');
        ta.remove();
        if (done) ok(); else btn.textContent = '複製失敗，請改按「存成檔案」';
      } catch (_) { btn.textContent = '複製失敗，請改按「存成檔案」'; }
    };
    try { navigator.clipboard.writeText(text).then(ok, fallback); } catch (_) { fallback(); }
  };
  var closed = false;
  var mount = function () {
    if (closed || !document.documentElement) return;
    if (panel) { if (!panel.isConnected) { try { document.documentElement.appendChild(panel); } catch (_) {} } return; }
    panel = mk('div', 'position:fixed;right:14px;bottom:14px;z-index:2147483647;width:400px;max-width:90vw;background:#fff;color:#111;font:13px/1.55 "Microsoft JhengHei","PingFang TC",sans-serif;border:2px solid #1d9bf0;border-radius:12px;padding:10px 12px;box-shadow:0 8px 28px rgba(0,0,0,.55);text-align:left;letter-spacing:0');
    panel.appendChild(mk('div', 'font-weight:700;font-size:14px;margin-bottom:2px', '工具箱：X 診斷中'));
    statusEl = mk('div', 'color:#333', '剛開始');
    panel.appendChild(statusEl);
    detailEl = mk('div', 'color:#555;font-size:12px;margin-top:4px;white-space:pre-wrap;word-break:break-all;max-height:180px;overflow:auto', '');
    panel.appendChild(detailEl);
    if (prev) detailEl.textContent = '上一次沒跑完（' + Math.round((Date.now() - prev.at) / 1000) + ' 秒前），最後有反應是在第 ' + prev.hb + ' 毫秒，當時正在做：' + (prev.cur || '?') + '\n之前做的事：\n' + (prev.marks || []).slice(-6).join('\n');
    document.documentElement.appendChild(panel);
  };
  var buttons = function () {
    if (!panel || panel.__btn) return;
    panel.__btn = true;
    var row = mk('div', 'display:flex;gap:8px;margin-top:8px;flex-wrap:wrap');
    var css = 'cursor:pointer;border:0;border-radius:999px;padding:6px 14px;font:inherit;font-weight:700;color:#fff;';
    var b1 = mk('button', css + 'background:#1d9bf0', '複製報告');
    b1.onclick = function (ev) { ev.stopPropagation(); latest = JSON.stringify(build('按了複製'), null, 1); copy(latest, b1); };
    var b2 = mk('button', css + 'background:#00ba7c', '存成檔案');
    b2.onclick = function (ev) { ev.stopPropagation(); latest = JSON.stringify(build('按了存檔'), null, 1); b2.textContent = download(latest) ? '已存到「下載」' : '存不了，請改按「複製報告」'; };
    var b3 = mk('button', css + 'background:#8b98a5', '關閉');
    b3.onclick = function (ev) { ev.stopPropagation(); closed = true; panel.remove(); };
    row.appendChild(b1); row.appendChild(b2); row.appendChild(b3);
    panel.appendChild(row);
  };
  var tick = function () {
    mount();
    if (!statusEl) return;
    var s = Math.round(now() / 100) / 10;
    if (panel.__failed) return;
    if (!panel.__done && bb.done) statusEl.textContent = '報告好了，但沒辦法自動存檔。請按下面的「存成檔案」或「複製報告」。';
    else if (!panel.__done) statusEl.textContent = '已經 ' + s + ' 秒。20 秒時會自動把報告存到「下載」。';
    else statusEl.textContent = '報告已存到「下載」（' + panel.__done + '）。不用做別的事，回去跟 Claude 說「好了」。';
  };
  // 一有 <html> 就放上去（之後網頁就算整個卡死，也看得到上面的字）
  if (document.documentElement) mount();
  else { try { var mo = new MutationObserver(function () { if (document.documentElement) { mo.disconnect(); mount(); } }); mo.observe(document, { childList: true }); } catch (_) {} }
  setInterval(tick, 500);
  var finish = function (why, auto) {
    try {
      latest = JSON.stringify(build(why), null, 1);
      if (auto) { var saved = download(latest); if (saved && panel) panel.__done = saved; }
      bb.done = true;
      bbSave();
      window.__mbOnRec = null;
      buttons();
      tick();
    } catch (e) { if (panel) panel.__failed = true; if (statusEl) statusEl.textContent = '報告做不出來：' + String(e).slice(0, 120); buttons(); }
  };
  if (prev && Date.now() - prev.at >= 25000) setTimeout(function () {
    try {
      var early = { 報告: 'MAOBOO X 診斷 ' + VER + '（上一次沒跑完的記錄）', 時間: new Date().toISOString(), 執行的地方: host, 瀏覽器: { UA: navigator.userAgent, 版本: uaFull }, 上次沒跑完: prevInfo(), 這次的網址: short(location.href) };
      var saved = download(JSON.stringify(early, null, 1), 'prev');
      if (saved && detailEl) detailEl.textContent = '上一次的記錄已存到「下載」（' + saved + '）。\n' + detailEl.textContent;
    } catch (_) {}
  }, 350);
  setTimeout(function () { finish('20 秒到了', true); }, 20000);
})();

(function () {
  if (window.__mbDiag) return;
  window.__mbDiag = true;
  var H = window.__mbHang || '';
  // RAW：不改 Worker 的程式，只在旁邊看（網址後面加 #mbraw）
  var RAW = !!window.__mbRaw || /mbraw/.test(location.hash);
  var sum = function (m) {
    try {
      if (m === null || m === undefined) return String(m);
      if (typeof m === 'string') return 'str(' + m.length + '):' + m.slice(0, 40);
      if (typeof m !== 'object') return typeof m + ':' + String(m).slice(0, 30);
      if (m instanceof ArrayBuffer) return 'ArrayBuffer(' + m.byteLength + ')';
      if (ArrayBuffer.isView(m)) return 'view(' + m.byteLength + ')';
      if (Array.isArray(m)) return 'arr(' + m.length + ')[' + m.slice(0, 4).map(function (x) { return typeof x === 'object' ? 'obj' : String(x).slice(0, 16); }).join(',') + ']';
      return '{' + Object.keys(m).slice(0, 8).map(function (k) { var v = m[k]; return k + ':' + (typeof v === 'string' ? 's' + v.length : typeof v === 'object' && v ? (Array.isArray(v) ? 'a' + v.length : 'o') : String(v).slice(0, 12)); }).join(',') + '}';
    } catch (_) { return '?'; }
  };
  var L = window.__mbLog = [];
  var t0 = performance.now();
  var now = function () { return Math.round(performance.now() - t0); };
  var rec = function (name) { var e = { n: name, t: now(), d: null }; if (L.length < 500) L.push(e); try { if (window.__mbOnRec) window.__mbOnRec(e); } catch (_) {} return e; };
  var arg = function (a) {
    try {
      if (typeof a === 'string') return a.replace(/^https?:\/\//, '').split('?')[0].slice(0, 56);
      if (a && a.url) return String(a.url).replace(/^https?:\/\//, '').split('?')[0].slice(0, 56);
      if (a && a.name) return String(a.name).slice(0, 30);
      return a === undefined ? '' : typeof a;
    } catch (_) { return '?'; }
  };
  var wrap = function (obj, key, name, hang) {
    try {
      if (!obj || typeof obj[key] !== 'function') return;
      var orig = obj[key];
      obj[key] = function () {
        var e = rec(name + '(' + arg(arguments[0]) + ')');
        if (hang) { e.d = 'STUB:never'; return new Promise(function () {}); }
        var r;
        try { r = orig.apply(this, arguments); } catch (x) { e.d = 'throw ' + String(x).slice(0, 60); throw x; }
        if (r && typeof r.then === 'function') {
          r.then(function () { e.d = 'ok@' + now(); }, function (x) { e.d = 'rej@' + now() + ' ' + String(x).slice(0, 60); });
        } else e.d = 'sync';
        return r;
      };
    } catch (_) {}
  };
  var proto = function (o) { return o ? Object.getPrototypeOf(o) : null; };
  if (navigator.gpu) wrap(proto(navigator.gpu), 'requestAdapter', 'gpu.requestAdapter', H === 'gpu');
  else rec('navigator.gpu 不存在').d = '-';
  if (window.GPUAdapter) { wrap(GPUAdapter.prototype, 'requestDevice', 'gpu.requestDevice', H === 'gpudev'); wrap(GPUAdapter.prototype, 'requestAdapterInfo', 'gpu.requestAdapterInfo', false); }
  if (navigator.serviceWorker) {
    var sp = proto(navigator.serviceWorker);
    wrap(sp, 'register', 'sw.register', H === 'sw'); wrap(sp, 'getRegistration', 'sw.getRegistration', H === 'sw'); wrap(sp, 'getRegistrations', 'sw.getRegistrations', H === 'sw');
  } else rec('navigator.serviceWorker 不存在').d = '-';
  if (navigator.locks) wrap(proto(navigator.locks), 'request', 'locks.request', H === 'locks');
  if (window.caches) { var cp = proto(window.caches); wrap(cp, 'open', 'caches.open', H === 'caches'); wrap(cp, 'match', 'caches.match', H === 'caches'); wrap(cp, 'keys', 'caches.keys', H === 'caches'); }
  if (navigator.storage) { var st = proto(navigator.storage); wrap(st, 'estimate', 'storage.estimate', H === 'storage'); wrap(st, 'persist', 'storage.persist', H === 'storage'); wrap(st, 'persisted', 'storage.persisted', H === 'storage'); wrap(st, 'getDirectory', 'storage.getDirectory', H === 'storage'); }
  if (navigator.permissions) wrap(proto(navigator.permissions), 'query', 'permissions.query', H === 'perm');
  if (navigator.credentials) wrap(proto(navigator.credentials), 'get', 'credentials.get', H === 'cred');
  wrap(Document.prototype, 'requestStorageAccess', 'requestStorageAccess', H === 'rsa');
  wrap(Document.prototype, 'hasStorageAccess', 'hasStorageAccess', H === 'rsa');
  if (navigator.userAgentData) wrap(proto(navigator.userAgentData), 'getHighEntropyValues', 'uaData.getHighEntropyValues', H === 'uadata');
  if (navigator.mediaDevices) wrap(proto(navigator.mediaDevices), 'enumerateDevices', 'mediaDevices.enumerateDevices', H === 'media');
  if (window.Notification) wrap(Notification, 'requestPermission', 'Notification.requestPermission', H === 'notif');
  wrap(Navigator.prototype, 'getBattery', 'getBattery', H === 'battery');
  if (navigator.keyboard) wrap(proto(navigator.keyboard), 'getLayoutMap', 'keyboard.getLayoutMap', H === 'kbd');
  if (window.OfflineAudioContext) wrap(OfflineAudioContext.prototype, 'startRendering', 'audio.startRendering', H === 'audio');
  if (document.fonts) wrap(proto(document.fonts), 'load', 'fonts.load', H === 'fonts');
  if (window.IDBFactory) {
    var io = IDBFactory.prototype.open;
    IDBFactory.prototype.open = function (n) {
      var e = rec('idb.open(' + n + ')');
      if (H === 'idb') {
        e.d = 'STUB:never';
        var fake = { addEventListener: function () {}, removeEventListener: function () {} };
        ['onsuccess', 'onerror', 'onupgradeneeded', 'onblocked'].forEach(function (k) { Object.defineProperty(fake, k, { set: function () {}, get: function () { return null; } }); });
        return fake;
      }
      var r = io.apply(this, arguments);
      r.addEventListener('success', function () { e.d = 'ok@' + now(); });
      r.addEventListener('error', function () { e.d = 'error@' + now(); });
      r.addEventListener('blocked', function () { e.d = 'blocked@' + now(); });
      return r;
    };
  }
  // Worker 的原始碼：從 Blob 建立時就記下來（之後建 Worker 時才拿得到）
  var OB = window.Blob, BT = new WeakMap(), BU = window.__mbBlobs = {};
  try {
    var NB = function (parts, opts) {
      var b = arguments.length > 1 ? new OB(parts, opts) : arguments.length ? new OB(parts) : new OB();
      try {
        if (parts && parts.length && parts.length < 64) {
          var ok = true, s = '';
          for (var i = 0; i < parts.length; i++) {
            var p = parts[i];
            if (typeof p === 'string') s += p;
            else if (p instanceof ArrayBuffer || ArrayBuffer.isView(p)) s += new TextDecoder().decode(p);
            else if (p instanceof OB && BT.has(p)) s += BT.get(p);
            else { ok = false; break; }
            if (s.length > 4000000) { ok = false; break; }
          }
          if (ok) BT.set(b, s);
        }
      } catch (_) {}
      return b;
    };
    NB.prototype = OB.prototype;
    if (!window.__mbLite) window.Blob = NB;
  } catch (_) {}
  var cou = URL.createObjectURL;
  if (!window.__mbLite) URL.createObjectURL = function (b) {
    var u = cou.apply(this, arguments);
    try { if (BT.has(b)) BU[u] = BT.get(b); } catch (_) {}
    return u;
  };
  // 放進 Worker 最前面的程式：記下 Worker 裡面在等哪些功能，定時回報給網頁
  var PRE = function () {
    try {
      var L = [], t0 = performance.now(), beat = 0;
      var now = function () { return Math.round(performance.now() - t0); };
      var pm = self.postMessage.bind(self);
      var flush = function () { try { pm({ __mbW: 1, beat: beat, t: now(), log: L.map(function (e) { return e.n + ' → ' + e.d + ' [' + e.t + ']'; }) }); } catch (_) {} };
      var rec = function (n) { var e = { n: n, t: now(), d: null }; if (L.length < 60) L.push(e); flush(); return e; };
      var done = function (e, d) { e.d = d; flush(); };
      var sum = __SUM__;
      var arg = function (a) {
        try {
          if (typeof a === 'string') return a.replace(/^https?:\/\//, '').slice(0, 50);
          if (a && a.url) return String(a.url).replace(/^https?:\/\//, '').slice(0, 50);
          if (a && a.name) return String(a.name).slice(0, 30);
          return a === undefined ? '' : typeof a;
        } catch (_) { return '?'; }
      };
      var wrap = function (obj, key, name) {
        try {
          if (!obj || typeof obj[key] !== 'function') return;
          var orig = obj[key];
          obj[key] = function () {
            var e = rec(name + '(' + arg(arguments[0]) + ')');
            var r;
            try { r = orig.apply(this, arguments); } catch (x) { done(e, 'throw ' + String(x).slice(0, 60)); throw x; }
            if (r && typeof r.then === 'function') r.then(function () { done(e, 'ok@' + now()); }, function (x) { done(e, 'rej@' + now() + ' ' + String(x).slice(0, 60)); });
            else done(e, r === null ? 'null' : 'sync');
            return r;
          };
        } catch (_) {}
      };
      var proto = function (o) { return o ? Object.getPrototypeOf(o) : null; };
      var nav = self.navigator || {};
      rec('START').d = '-';
      wrap(self, 'fetch', 'fetch'); wrap(self, 'importScripts', 'importScripts'); wrap(self, 'createImageBitmap', 'createImageBitmap');
      if (nav.gpu) wrap(proto(nav.gpu), 'requestAdapter', 'gpu.requestAdapter'); else rec('Worker 裡沒有 navigator.gpu').d = '-';
      if (self.GPUAdapter) { wrap(GPUAdapter.prototype, 'requestDevice', 'gpu.requestDevice'); wrap(GPUAdapter.prototype, 'requestAdapterInfo', 'gpu.requestAdapterInfo'); }
      if (nav.storage) { var sp = proto(nav.storage); wrap(sp, 'estimate', 'storage.estimate'); wrap(sp, 'getDirectory', 'storage.getDirectory'); wrap(sp, 'persisted', 'storage.persisted'); }
      if (nav.permissions) wrap(proto(nav.permissions), 'query', 'permissions.query');
      if (nav.userAgentData) wrap(proto(nav.userAgentData), 'getHighEntropyValues', 'uaData.getHighEntropyValues');
      if (nav.locks) wrap(proto(nav.locks), 'request', 'locks.request');
      if (nav.mediaCapabilities) { wrap(proto(nav.mediaCapabilities), 'decodingInfo', 'mediaCap.decodingInfo'); wrap(proto(nav.mediaCapabilities), 'encodingInfo', 'mediaCap.encodingInfo'); }
      ['hid', 'usb', 'serial'].forEach(function (k) { try { if (nav[k]) { wrap(proto(nav[k]), 'getDevices', k + '.getDevices'); wrap(proto(nav[k]), 'getPorts', k + '.getPorts'); } } catch (_) {} });
      if (self.caches) { var cp = proto(self.caches); wrap(cp, 'open', 'caches.open'); wrap(cp, 'keys', 'caches.keys'); wrap(cp, 'match', 'caches.match'); wrap(cp, 'has', 'caches.has'); }
      if (self.OffscreenCanvas) { wrap(OffscreenCanvas.prototype, 'getContext', 'offscreen.getContext'); wrap(OffscreenCanvas.prototype, 'convertToBlob', 'offscreen.convertToBlob'); }
      if (self.crypto && self.crypto.subtle) ['digest', 'importKey', 'sign', 'encrypt', 'decrypt', 'deriveBits', 'deriveKey', 'generateKey', 'exportKey'].forEach(function (k) { wrap(proto(self.crypto.subtle), k, 'subtle.' + k); });
      if (self.WebAssembly) { wrap(WebAssembly, 'instantiate', 'wasm.instantiate'); wrap(WebAssembly, 'compile', 'wasm.compile'); wrap(WebAssembly, 'instantiateStreaming', 'wasm.instantiateStreaming'); }
      if (self.Atomics) wrap(Atomics, 'wait', 'Atomics.wait');
      if (self.FontFace) wrap(FontFace.prototype, 'load', 'FontFace.load');
      if (self.IDBFactory) {
        var io = IDBFactory.prototype.open;
        IDBFactory.prototype.open = function (n) {
          var e = rec('idb.open(' + n + ')');
          var r = io.apply(this, arguments);
          r.addEventListener('success', function () { done(e, 'ok@' + now()); });
          r.addEventListener('error', function () { done(e, 'error@' + now()); });
          r.addEventListener('blocked', function () { done(e, 'blocked@' + now()); });
          return r;
        };
        wrap(IDBFactory.prototype, 'databases', 'idb.databases');
      }
      if (self.XMLHttpRequest) {
        var xo = XMLHttpRequest.prototype.open, xs = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.open = function (m, u, a) { this.__mbU = arg(String(u)) + (a === false ? ' 同步' : ''); return xo.apply(this, arguments); };
        XMLHttpRequest.prototype.send = function () { var e = rec('xhr(' + this.__mbU + ')'); this.addEventListener('loadend', function () { done(e, this.status + '@' + now()); }); return xs.apply(this, arguments); };
      }
      ['WebSocket', 'Worker', 'BroadcastChannel', 'EventSource', 'SharedArrayBuffer'].forEach(function (k) {
        try {
          var O = self[k];
          if (typeof O !== 'function') return;
          var N = function (a, b) { rec('new ' + k + '(' + arg(String(a)) + ')').d = '-'; return arguments.length > 1 ? new O(a, b) : arguments.length ? new O(a) : new O(); };
          N.prototype = O.prototype;
          self[k] = N;
        } catch (_) {}
      });
      self.postMessage = function (m) { rec('postMessage(' + sum(m) + ')').d = 'sent'; flush(); return pm.apply(null, arguments); };
      self.addEventListener('message', function (ev) { rec('onmessage(' + sum(ev.data) + ')').d = '-'; });
      self.addEventListener('error', function (ev) { rec('ERROR ' + String(ev.message).slice(0, 90)).d = '-'; });
      self.addEventListener('unhandledrejection', function (ev) { rec('UNHANDLED ' + String(ev.reason).slice(0, 90)).d = '-'; });
      setInterval(function () { beat++; flush(); }, 700);
    } catch (x) { try { self.postMessage({ __mbW: 1, beat: -1, t: 0, log: ['PRELUDE 壞掉 ' + String(x).slice(0, 80)] }); } catch (_) {} }
  };
  var PRE_SRC = '(' + PRE.toString().replace('__SUM__', sum.toString()) + ')();\n';
  var WK = window.__mbWorkers = [];
  if (window.Worker) {
    var W = window.Worker;
    var NW = function (u, o) {
      var su = String(u);
      var e = rec('new Worker(' + su.slice(0, 24) + ')');
      var info = { u: su.slice(-12), len: -1, head: '', inst: false, stack: '', posts: [], got: [], term: null, beat: null, wt: null, w: null };
      if (WK.length < 8) WK.push(info);
      try { info.stack = String(new Error().stack).split('\n').slice(2, 6).map(function (l) { return l.replace(/^\s*at\s+/, '').replace(/https?:\/\/[^\/]+/g, '').slice(-64); }).join(' < '); } catch (_) {}
      var src = BU[su];
      if (typeof src === 'string') { info.len = src.length; info.head = src.slice(0, 140); }
      if (H === 'worker') {
        e.d = 'STUB:silent';
        return { postMessage: function () {}, terminate: function () {}, addEventListener: function () {}, removeEventListener: function () {}, dispatchEvent: function () { return true; }, onmessage: null, onerror: null, onmessageerror: null };
      }
      var real = u;
      if (!RAW && typeof src === 'string') {
        try { real = cou.call(URL, new OB([PRE_SRC, src], { type: 'text/javascript' })); info.inst = true; } catch (_) { real = u; }
      }
      var w = arguments.length > 1 ? new W(real, o) : new W(real);
      w.addEventListener('message', function (ev) {
        var d = ev.data;
        if (d && d.__mbW === 1) { ev.stopImmediatePropagation(); info.w = d.log; info.beat = d.beat; info.wt = d.t; return; }
        if (!e.d) e.d = 'msg@' + now();
        if (info.got.length < 6) info.got.push(now() + ':' + sum(d));
      });
      w.addEventListener('error', function (x) { e.d = 'error@' + now() + ' ' + ((x && x.message) || ''); if (info.got.length < 8) info.got.push(now() + ':ERROR ' + ((x && x.message) || '')); });
      w.addEventListener('messageerror', function () { if (info.got.length < 8) info.got.push(now() + ':messageerror'); });
      var pmw = w.postMessage, tm = w.terminate;
      w.postMessage = function (m) { if (info.posts.length < 6) info.posts.push(now() + ':' + sum(m)); return pmw.apply(this, arguments); };
      w.terminate = function () { info.term = now(); return tm.apply(this, arguments); };
      return w;
    };
    NW.prototype = W.prototype;
    window.Worker = NW;
  }
  if (window.fetch) {
    var F = window.fetch;
    window.fetch = function (i) {
      var e = rec('fetch(' + arg(i) + ')');
      var r = F.apply(this, arguments);
      r.then(function (v) {
        e.d = v.status + '@' + now();
        try {
          if (window.__mbLite && /(^|\.)x\.com\//.test(e.n.slice(6)) && !/\.(mp4|m4s|ts|m3u8)/.test(e.n)) {
            v.clone().arrayBuffer().then(function (b) { e.d += ' 內容 ' + b.byteLength + 'B@' + now(); }, function (x) { e.d += ' 內容讀不完@' + now() + ' ' + String(x).slice(0, 30); });
          }
        } catch (_) {}
      }, function (x) { e.d = 'rej@' + now() + ' ' + String(x).slice(0, 40); });
      return r;
    };
  }
  var xo = XMLHttpRequest.prototype.open, xs = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (m, u) { this.__mbU = arg(String(u)); return xo.apply(this, arguments); };
  XMLHttpRequest.prototype.send = function () { var e = rec('xhr(' + this.__mbU + ')'); this.addEventListener('loadend', function () { e.d = this.status + '@' + now(); }); return xs.apply(this, arguments); };
  var gc = HTMLCanvasElement.prototype.getContext;
  var c2d = 0;
  HTMLCanvasElement.prototype.getContext = function (k) {
    if (k === '2d' && ++c2d > 3) return gc.apply(this, arguments);
    var e = rec('canvas.getContext(' + k + ')');
    if (H === 'webgl' && /webgl|webgpu/.test(String(k))) { e.d = 'STUB:null'; return null; }
    var r = gc.apply(this, arguments);
    e.d = r ? 'ok' : 'null';
    return r;
  };
  if (window.OffscreenCanvas) {
    var oc = OffscreenCanvas.prototype.getContext;
    OffscreenCanvas.prototype.getContext = function (k) { var e = rec('offscreen.getContext(' + k + ')'); var r = oc.apply(this, arguments); e.d = r ? 'ok' : 'null'; return r; };
  }
  var rafN = 0, ra = window.requestAnimationFrame;
  window.requestAnimationFrame = function (cb) { rafN++; return H === 'raf' ? 0 : ra.call(window, cb); };
  var AN = { start: 0, end: 0, cancel: 0, tend: 0, waapi: 0, names: [] };
  document.addEventListener('animationstart', function (ev) { AN.start++; if (AN.names.length < 6 && AN.names.indexOf(ev.animationName) < 0) AN.names.push(ev.animationName); }, true);
  document.addEventListener('animationend', function () { AN.end++; }, true);
  document.addEventListener('animationcancel', function () { AN.cancel++; }, true);
  document.addEventListener('transitionend', function () { AN.tend++; }, true);
  try {
    var ea = Element.prototype.animate;
    if (ea) Element.prototype.animate = function () {
      var an = ea.apply(this, arguments);
      try {
        if (++AN.waapi <= 6) { var e = rec('el.animate(' + (this.tagName || '') + ')'); an.finished.then(function () { e.d = 'ok@' + now(); }, function (x) { e.d = 'rej@' + now() + ' ' + String(x).slice(0, 30); }); }
      } catch (_) {}
      return an;
    };
    var vt = Document.prototype.startViewTransition;
    if (vt) Document.prototype.startViewTransition = function () {
      var e = rec('startViewTransition()');
      var t = vt.apply(this, arguments);
      try { t.finished.then(function () { e.d = 'ok@' + now(); }, function (x) { e.d = 'rej@' + now() + ' ' + String(x).slice(0, 40); }); } catch (_) {}
      return t;
    };
  } catch (_) {}
  wrap(HTMLMediaElement.prototype, 'play', 'media.play', false);
  if (window.requestIdleCallback) { var ric = window.requestIdleCallback, ricN = 0, ricD = 0; window.requestIdleCallback = function (cb, o) { ricN++; return ric.call(window, function (d) { ricD++; return cb(d); }, o); }; }
  var ce = console.error, ceN = 0;
  console.error = function () {
    try { if (++ceN <= 8) rec('console.error ' + Array.prototype.slice.call(arguments, 0, 3).map(function (x) { return String(x && x.message ? x.message : x).slice(0, 70); }).join(' | ')).d = '-'; } catch (_) {}
    return ce.apply(console, arguments);
  };
  window.addEventListener('error', function (ev) {
    var t = ev.target;
    rec('ERROR ' + (t && t.tagName ? t.tagName + ' ' + String(t.src || t.href || '').slice(-60) : String(ev.message).slice(0, 90))).d = '-';
  }, true);
  window.addEventListener('unhandledrejection', function (ev) { rec('UNHANDLED ' + String(ev.reason).slice(0, 90)).d = '-'; });
  window.__mbSummary = function () {
    var txt = document.body ? document.body.innerText.replace(/\s+/g, ' ') : '';
    return JSON.stringify({
      href: location.href, title: document.title, ready: document.readyState, vis: document.visibilityState, lang: navigator.language,
      dark: matchMedia('(prefers-color-scheme: dark)').matches, textLen: txt.length, text: txt.slice(0, 100), raf: rafN, hang: H,
      raw: RAW, canvas2d: c2d,
      motion: (function () {
        try {
          var st = {}, list = document.getAnimations ? document.getAnimations() : [];
          list.forEach(function (x) { st[x.playState] = (st[x.playState] || 0) + 1; });
          return {
            reduce: matchMedia('(prefers-reduced-motion: reduce)').matches, attr: document.documentElement.getAttribute('data-reduce-motion'),
            ev: AN, now: st, first: list.slice(0, 4).map(function (x) { return (x.animationName || x.transitionProperty || x.id || 'waapi') + ':' + x.playState + '@' + Math.round(x.currentTime || 0); }),
            ric: (typeof ricN === 'number' ? ricN + '/' + ricD : ''),
            videos: Array.prototype.map.call(document.querySelectorAll('video'), function (v) { return (v.paused ? 'paused' : 'playing') + ' rs' + v.readyState + ' t' + Math.round(v.currentTime * 10) / 10; }).slice(0, 4),
          };
        } catch (x) { return { err: String(x) }; }
      })(),
      view: { dpr: window.devicePixelRatio, inner: innerWidth + 'x' + innerHeight, els: document.querySelectorAll('*').length, body: document.body ? document.body.children.length : -1 },
      env: (function () {
        try {
          return {
            ua: navigator.userAgent.replace(/^Mozilla\/5\.0 \(Windows NT 10\.0; Win64; x64\) AppleWebKit\/537\.36 \(KHTML, like Gecko\) /, ''),
            brands: navigator.userAgentData ? navigator.userAgentData.brands.map(function (b) { return b.brand + ' ' + b.version; }).join(',') : '',
            webview: !!(window.chrome && window.chrome.webview), tauri: Object.keys(window).filter(function (k) { return /TAURI|^ipc$/i.test(k); }).join(','),
            focus: document.hasFocus(), wd: navigator.webdriver, hc: navigator.hardwareConcurrency, mem: navigator.deviceMemory, plugins: navigator.plugins.length,
            notif: window.Notification ? Notification.permission : 'none', coi: !!self.crossOriginIsolated, sab: typeof SharedArrayBuffer, langs: navigator.languages.join(','),
          };
        } catch (x) { return { err: String(x) }; }
      })(),
      workers: WK.map(function (k) { return { u: k.u, len: k.len, inst: k.inst, stack: k.stack, posts: k.posts, got: k.got, term: k.term, beat: k.beat, wt: k.wt, w: k.w, head: k.head }; }),
      pending: L.filter(function (e) { return !e.d; }).map(function (e) { return e.n + ' [' + e.t + ']'; }).slice(0, 30),
      calls: L.map(function (e) { return e.n + ' → ' + e.d + ' [' + e.t + ']'; }).slice(0, 110),
    });
  };
})();
