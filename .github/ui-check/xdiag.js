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
  var rec = function (name) { var e = { n: name, t: now(), d: null }; if (L.length < 500) L.push(e); return e; };
  var arg = function (a) {
    try {
      if (typeof a === 'string') return a.replace(/^https?:\/\//, '').slice(0, 56);
      if (a && a.url) return String(a.url).replace(/^https?:\/\//, '').slice(0, 56);
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
    window.Blob = NB;
  } catch (_) {}
  var cou = URL.createObjectURL;
  URL.createObjectURL = function (b) {
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
      r.then(function (v) { e.d = v.status + '@' + now(); }, function (x) { e.d = 'rej@' + now() + ' ' + String(x).slice(0, 40); });
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
