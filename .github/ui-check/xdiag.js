(function () {
  if (window.__mbDiag) return;
  window.__mbDiag = true;
  var H = window.__mbHang || '';
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
  if (window.Worker) {
    var W = window.Worker;
    var NW = function (u, o) {
      var e = rec('new Worker(' + String(u).slice(0, 24) + ')');
      if (H === 'worker') {
        e.d = 'STUB:silent';
        return { postMessage: function () {}, terminate: function () {}, addEventListener: function () {}, removeEventListener: function () {}, dispatchEvent: function () { return true; }, onmessage: null, onerror: null, onmessageerror: null };
      }
      var w = new W(u, o);
      w.addEventListener('message', function () { if (!e.d) e.d = 'msg@' + now(); });
      w.addEventListener('error', function (x) { e.d = 'error@' + now() + ' ' + ((x && x.message) || ''); });
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
  HTMLCanvasElement.prototype.getContext = function (k) {
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
      pending: L.filter(function (e) { return !e.d; }).map(function (e) { return e.n + ' [' + e.t + ']'; }).slice(0, 30),
      calls: L.map(function (e) { return e.n + ' → ' + e.d + ' [' + e.t + ']'; }).slice(0, 140),
    });
  };
})();
