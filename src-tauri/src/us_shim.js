(() => {
  "use strict";
  // 使用者腳本的執行環境（每份文件一開始就跑，內嵌框架也有）。
  // 這一段只負責準備：腳本本身由後面各自登記的小段程式交進來（add），最後由「帶最新設定值」的那一段啟動（go）。
  const W = window;
  const SLOT = "__SLOT__";
  // 這個視窗物件已經跑過一次了（空白頁沿用下來的視窗：裡面的東西可能被開它的網頁動過）→ 什麼都不做
  if (SLOT in W) return;
  const D = W.document, L = W.location;
  // 程式自己的頁面不碰
  if (L.hostname === "__APP_HOST__") return;
  const TOKEN = "__TOKEN__", DOWN = "__DOWN__";
  const TOP = W.top === W;

  // 先把會用到的內建功能抓好：之後網頁就算把它們換掉，也影響不到跟程式之間的訊息
  const jparse = JSON.parse, jstr = JSON.stringify;
  const keysOf = Object.keys, defProp = Object.defineProperty, freeze = Object.freeze, getDesc = Object.getOwnPropertyDescriptor;
  const apply = Reflect.apply;
  const setT = W.setTimeout.bind(W);
  const U8 = W.Uint8Array, PromiseC = W.Promise, SetC = W.Set;
  const b64dec = W.atob.bind(W), b64enc = W.btoa.bind(W);
  const fromChar = String.fromCharCode;
  const cerr = console.error.bind(console);
  let dataGet = null;
  try { dataGet = getDesc(W.MessageEvent.prototype, "data").get; } catch (_) {}

  /* ---------- 跟程式互傳訊息的通道 ---------- */
  // 網頁自己的程式碰不到這個通道：先抓起來，再從網頁上拿掉（不能假裝成腳本傳訊息給程式，也看不到程式回的內容）
  const C = W.chrome;
  const wv = C && C.webview;
  let post = null, listen = null;
  if (wv && typeof wv.postMessage === "function") {
    post = wv.postMessage.bind(wv);
    listen = wv.addEventListener.bind(wv);
    try { delete C.webview; } catch (_) {}
    if (C.webview) { try { defProp(C, "webview", { value: undefined, configurable: true, writable: true }); } catch (_) {} }
  }
  // 只在一般網頁（http、https、本機檔案）執行腳本
  if (!/^(https?|file):$/.test(L.protocol)) {
    // 空白頁的視窗之後可能被同一個網站的網頁沿用：留一個拿不掉的記號，沿用時就不會再跑（上面第一行）
    if (L.href === "about:blank") { try { defProp(W, SLOT, { value: freeze({ __proto__: null }) }); } catch (_) {} }
    return;
  }
  // 訊息一律是文字，而且權杖不放進物件裡（物件會經過可能被網頁動過手腳的轉換）。
  // 外面那一層長得像程式內部指令，但鑰匙是空的：程式內部會直接略過，只有使用者腳本這邊認得
  const PRE = '{"cmd":"mb","callback":0,"error":0,"payload":null,"__TAURI_INVOKE_KEY__":"","mb":"' + TOKEN + '","d":';
  const send = (m) => {
    if (!post) return false;
    try {
      // 換成沒有原型的物件再轉成文字（網頁在 Object.prototype 上動手腳也看不到內容）
      const c = { __proto__: null }, ks = keysOf(m);
      for (let i = 0; i < ks.length; i++) c[ks[i]] = m[ks[i]];
      post(PRE + jstr(jstr(c)) + "}");
      return true;
    } catch (_) { return false; }
  };
  if (listen) {
    listen("message", (e) => {
      let d;
      try { d = dataGet ? apply(dataGet, e, []) : e.data; } catch (_) { try { d = e.data; } catch (__) {} }
      if (d && d.mb === DOWN) { try { onApp(d); } catch (err) { cerr("[使用者腳本]", err); } }
    });
  }

  /* ---------- 腳本交進來、啟動 ---------- */
  let defs = [];
  let baked = null, started = false;
  // 設定值是一整段 JSON 文字：這份文件真的有腳本要跑才解開（大部分的內嵌框架用不到，不用白花時間）
  const start = (ver, raw) => {
    if (started) return;
    started = true;
    try { delete W[SLOT]; } catch (_) {}
    const b = typeof raw === "string" ? { ver, raw } : (baked || { ver: 0, raw: "{}" });
    try { run(b.ver, () => { try { return jparse(b.raw) || {}; } catch (_) { return {}; } }); } catch (e) { cerr("[使用者腳本] 啟動失敗", e); }
  };
  try {
    defProp(W, SLOT, {
      configurable: true,
      value: freeze({
        __proto__: null,
        add: (tok, d) => { if (tok === TOKEN && !started && d && typeof d.fn === "function") defs.push(d); },
        // 腳本清單在這個分頁建立之後變過：前面交進來的作廢，改用後面交的
        reset: (tok) => { if (tok === TOKEN && !started) defs = []; },
        // 建立分頁時先放的一份。分頁才剛建立（幾秒內）：這一份就是最新的，直接啟動
        // （「帶最新設定值啟動」那一段這時候可能還沒登記好）；過了這段時間就只留著備用
        baked: (tok, ver, raw, made) => {
          if (tok !== TOKEN || started) return;
          baked = { ver, raw };
          if (typeof made === "number" && Date.now() - made < 4000) start(ver, raw);
        },
        go: (tok, ver, raw) => { if (tok === TOKEN) start(ver, raw); },
      }),
    });
  } catch (_) { return; }
  // 備用：分頁剛建立的那一下，「帶最新設定值啟動」的那一段可能還沒登記好。網頁出現第一個元素時
  // （一定在網頁自己的程式之前）還沒啟動，就用建立分頁時的設定值啟動
  const fallback = () => start(0, null);
  try { new MutationObserver((_, ob) => { ob.disconnect(); fallback(); }).observe(D, { childList: true, subtree: true }); } catch (_) {}
  D.addEventListener("DOMContentLoaded", fallback, { once: true });

  /* ---------- 程式傳來的訊息 ---------- */
  const apis = { __proto__: null };   // 腳本編號 → 這份文件裡那支腳本的狀態
  const xhrs = { __proto__: null };   // 連線編號 → 進行中的 GM_xmlhttpRequest
  // 連線編號從亂數開始：上一份文件還沒結束的連線晚到的回應，不會剛好對上這份文件的編號
  let seq = 1000 + Math.floor(Math.random() * 1e9);
  function onApp(d) {
    if (d.ev === "val") {
      const a = apis[d.s];
      if (a) a.remote(d.k, d.v);
    } else if (d.ev === "delta") {
      // 這份文件啟動時拿到的設定值比較舊：補上之後變過的那幾個
      const all = d.vals || {};
      const ids = keysOf(all);
      for (let i = 0; i < ids.length; i++) {
        const a = apis[ids[i]], ch = all[ids[i]] || {};
        if (!a) continue;
        const ks = keysOf(ch);
        for (let j = 0; j < ks.length; j++) a.remote(ks[j], ch[ks[j]]);
      }
    } else if (d.ev === "sync") {
      // 這份文件啟動時拿到的設定值比較舊：整份對一次
      const all = d.vals || {};
      const ids = keysOf(all);
      for (let i = 0; i < ids.length; i++) { const a = apis[ids[i]]; if (a) a.sync(all[ids[i]] || {}); }
    } else if (d.ev === "menu") {
      const a = apis[d.s];
      if (a) a.menu(d.id);
    } else if (d.ev === "xhr") {
      const x = xhrs[d.rid];
      if (x) x.on(d);
    }
  }

  /* ---------- 位元組與文字 ---------- */
  const toBytes = (b64) => {
    const s = b64dec(b64), n = s.length, out = new U8(n);
    for (let i = 0; i < n; i++) out[i] = s.charCodeAt(i);
    return out;
  };
  const toB64 = (u8) => {
    let s = "";
    for (let i = 0; i < u8.length; i += 0x8000) s += fromChar.apply(null, u8.subarray(i, i + 0x8000));
    return b64enc(s);
  };
  const concat = (chunks, total) => {
    const out = new U8(total);
    let o = 0;
    for (const c of chunks) { out.set(c, o); o += c.length; }
    return out;
  };
  const charsetOf = (ct) => { const m = /charset\s*=\s*"?([^";\s]+)/i.exec(ct || ""); return m ? m[1] : ""; };
  const decode = (u8, cs) => {
    try { return new TextDecoder(cs || "utf-8").decode(u8); } catch (_) { return new TextDecoder("utf-8").decode(u8); }
  };
  const warn = (s, e) => cerr("[使用者腳本] " + s.name, e);

  /* ---------- 一支腳本的 GM_* 功能 ---------- */
  function makeApi(s, init) {
    const grants = new SetC(s.grants || []);
    const can = (...names) => names.some((n) => grants.has(n));
    const info = {
      script: {
        name: s.name, version: s.version, description: s.desc, namespace: s.ns, author: s.author, homepage: s.home, icon: s.icon,
        matches: s.matches, includes: s.includes, excludes: s.excludes, grant: s.grants, connects: s.conn,
        "run-at": s.at === "start" ? "document-start" : s.at === "body" ? "document-body" : s.at === "end" ? "document-end" : "document-idle",
        resources: keysOf(s.res || {}).map((n) => ({ name: n })), updateURL: s.upd, downloadURL: s.dl, unwrap: false,
      },
      scriptHandler: "MAOBOO", version: "2.0", scriptWillUpdate: !!(s.upd || s.dl), isIncognito: false,
    };

    // ---- 設定值：整個瀏覽器共用（不分網站），別的分頁改了這裡也會跟著變 ----
    const store = { __proto__: null };
    for (const k of keysOf(init)) if (typeof init[k] === "string") store[k] = init[k];
    const listeners = [];
    let lseq = 0;
    const val = (j, d) => { if (j === undefined || j === null) return d; try { return jparse(j); } catch (_) { return d; } };
    const fire = (k, oj, nj, remote) => {
      for (const l of listeners.slice()) {
        if (l.k !== k) continue;
        try { l.fn(k, val(oj, undefined), val(nj, undefined), remote); } catch (e) { warn(s, e); }
      }
    };
    const getValue = (k, d) => val(store[String(k)], d);
    const deleteValue = (k) => {
      k = String(k);
      const old = store[k];
      if (old === undefined) return;
      delete store[k];
      send({ op: "set", s: s.id, k, v: null });
      PromiseC.resolve().then(() => fire(k, old, undefined, false));
    };
    const setValue = (k, v) => {
      k = String(k);
      let j;
      try { j = jstr(v); } catch (_) { return; }
      if (j === undefined) { deleteValue(k); return; }
      const old = store[k];
      if (old === j) return;
      store[k] = j;
      send({ op: "set", s: s.id, k, v: j });
      PromiseC.resolve().then(() => fire(k, old, j, false));
    };
    const listValues = () => keysOf(store);
    const getValues = (q) => {
      const out = {};
      if (Array.isArray(q)) { for (const k of q) { const v = getValue(k); if (v !== undefined) out[k] = v; } }
      else if (q && typeof q === "object") { for (const k of keysOf(q)) out[k] = getValue(k, q[k]); }
      else { for (const k of keysOf(store)) out[k] = getValue(k); }
      return out;
    };
    const setValues = (o) => { if (o && typeof o === "object") for (const k of keysOf(o)) setValue(k, o[k]); };
    const deleteValues = (ks) => { if (Array.isArray(ks)) for (const k of ks) deleteValue(k); };
    const addListener = (k, fn) => { if (typeof fn !== "function") return 0; const id = ++lseq; listeners.push({ id, k: String(k), fn }); return id; };
    const removeListener = (id) => { const i = listeners.findIndex((l) => l.id === id); if (i >= 0) listeners.splice(i, 1); };
    const remote = (k, j) => {
      const old = store[k];
      if (j === null || j === undefined) { if (old === undefined) return; delete store[k]; fire(k, old, undefined, true); }
      else { if (old === j) return; store[k] = j; fire(k, old, j, true); }
    };

    // ---- 樣式、元素、附帶的資源 ----
    const addStyle = (css) => { const el = D.createElement("style"); el.textContent = css; (D.head || D.documentElement).appendChild(el); return el; };
    const addElement = (a, b, c) => {
      const parent = typeof a === "string" ? (D.head || D.body || D.documentElement) : a;
      const tag = typeof a === "string" ? a : b;
      const attrs = (typeof a === "string" ? b : c) || {};
      const el = D.createElement(tag);
      for (const k of keysOf(attrs)) { if (k === "textContent") el.textContent = attrs[k]; else el.setAttribute(k, attrs[k]); }
      parent.appendChild(el);
      return el;
    };
    const resText = (n) => { const r = (s.res || {})[n]; return r ? decode(toBytes(r.b), charsetOf(r.t)) : ""; };
    const resUrl = (n) => { const r = (s.res || {})[n]; return r ? "data:" + (r.t || "application/octet-stream") + ";base64," + r.b : ""; };

    // ---- 選單（顯示在工具列的「使用者腳本」按鈕裡）----
    const menus = { __proto__: null };
    let mseq = 0;
    const registerMenu = (caption, fn, o) => {
      if (!TOP) return undefined;
      const opt = typeof o === "string" ? { accessKey: o } : (o && typeof o === "object" ? o : {});
      const id = opt.id !== undefined && opt.id !== null ? String(opt.id) : "m" + (++mseq);
      menus[id] = typeof fn === "function" ? fn : () => {};
      send({ op: "menu", s: s.id, id, caption: String(caption), title: opt.title ? String(opt.title) : "", close: opt.autoClose !== false });
      return id;
    };
    const unregisterMenu = (id) => { id = String(id); if (menus[id]) { delete menus[id]; send({ op: "unmenu", s: s.id, id }); } };

    // ---- 連到別的網站（由程式代為連線，不受網頁的跨網站限制）----
    const xhr = (d) => {
      d = d || {};
      const rid = ++seq;
      const st = { done: false, chunks: [], loaded: 0, total: 0, head: null, stream: null, ctl: null };
      const call = (name, r) => { const f = d[name]; if (typeof f === "function") { try { f.call(d, r); } catch (e) { warn(s, e); } } };
      const rt = String(d.responseType || "").toLowerCase();
      const base = (ready) => {
        const h = st.head || {};
        return { finalUrl: h.url || "", readyState: ready, status: h.status || 0, statusText: h.statusText || "", responseHeaders: h.headers || "", context: d.context };
      };
      const finish = (kind) => {
        if (st.done) return;
        st.done = true;
        delete xhrs[rid];
        if (st.ctl) { try { kind === "load" ? st.ctl.close() : st.ctl.error(new Error(kind)); } catch (_) {} }
        if (kind === "load") {
          const r = base(4);
          const ct = d.overrideMimeType || (/^content-type:\s*(.*)$/im.exec(r.responseHeaders) || [])[1] || "";
          if (rt === "stream") r.response = st.stream;
          else {
            const bytes = concat(st.chunks, st.loaded);
            st.chunks = [];
            if (rt === "arraybuffer") r.response = bytes.buffer;
            else if (rt === "blob") r.response = new Blob([bytes], { type: ct.split(";")[0].trim() });
            else {
              const text = decode(bytes, charsetOf(ct));
              r.responseText = text;
              if (rt === "json") { try { r.response = jparse(text); } catch (_) { r.response = null; } }
              else if (rt === "document") { try { r.response = new DOMParser().parseFromString(text, /xml/i.test(ct) && !/html/i.test(ct) ? "text/xml" : "text/html"); r.responseXML = r.response; } catch (_) { r.response = null; } }
              else r.response = text;
            }
          }
          r.loaded = st.loaded; r.total = st.total || st.loaded; r.lengthComputable = true;
          call("onreadystatechange", r);
          call("onload", r);
          call("onloadend", r);
        } else {
          const r = base(4);
          r.error = kind;
          call(kind === "timeout" ? "ontimeout" : kind === "abort" ? "onabort" : "onerror", r);
          call("onloadend", r);
        }
      };
      const fail = (kind) => PromiseC.resolve().then(() => finish(kind));
      const self = {
        on: (m) => {
          if (st.done) return;
          if (m.t === "head") {
            st.head = m; st.total = m.total > 0 ? m.total : 0;
            const r = base(2);
            if (rt === "stream") {
              try { st.stream = new ReadableStream({ start: (c) => { st.ctl = c; }, cancel: () => self.abort() }); } catch (_) {}
              r.response = st.stream;
            }
            call("onreadystatechange", r);
            call("onloadstart", r);
          } else if (m.t === "data") {
            const u8 = toBytes(m.b);
            st.loaded += u8.length;
            if (st.ctl) { try { st.ctl.enqueue(u8); } catch (_) {} } else st.chunks.push(u8);
            const r = base(3);
            r.loaded = st.loaded; r.total = st.total; r.lengthComputable = st.total > 0; r.done = st.loaded; r.position = st.loaded; r.totalSize = st.total;
            if (rt === "stream") r.response = st.stream;
            call("onprogress", r);
          } else if (m.t === "end") finish("load");
          else if (m.t === "err") finish(m.kind === "timeout" ? "timeout" : m.kind === "abort" ? "abort" : "error");
        },
        abort: () => { if (!st.done) { send({ op: "xhr-abort", rid }); finish("abort"); } },
      };
      let url;
      try { url = new URL(String(d.url), L.href).href; } catch (_) { fail("error"); return { abort: () => {} }; }
      // 網頁自己產生的資料（blob:、data:）只有網頁讀得到
      if (/^(blob|data):/i.test(url)) {
        fetch(url).then((r) => r.arrayBuffer()).then((buf) => {
          self.on({ t: "head", status: 200, statusText: "OK", headers: "", url, total: buf.byteLength });
          self.on({ t: "data", b: toB64(new U8(buf)) });
          self.on({ t: "end" });
        }, () => finish("error"));
        return { abort: self.abort };
      }
      if (!/^https?:/i.test(url) || !post) { fail("error"); return { abort: () => {} }; }
      xhrs[rid] = self;
      const headers = { __proto__: null };
      if (d.headers && typeof d.headers === "object") for (const k of keysOf(d.headers)) headers[String(k)] = String(d.headers[k]);
      if (d.cookie) headers["X-MB-Cookie"] = String(d.cookie);
      const go = (body, ctype) => {
        if (st.done) return;
        if (ctype && !keysOf(headers).some((k) => k.toLowerCase() === "content-type")) headers["Content-Type"] = ctype;
        const ok = send({
          op: "xhr", s: s.id, rid, url, method: String(d.method || "GET").toUpperCase(), headers, body,
          timeout: +d.timeout > 0 ? +d.timeout : 0, anon: !!d.anonymous, ua: navigator.userAgent, redirect: String(d.redirect || ""),
          nocache: !!d.nocache,
          user: d.user !== undefined && d.user !== null ? String(d.user) : null, password: d.password !== undefined && d.password !== null ? String(d.password) : null,
        });
        if (!ok) finish("error");
      };
      const data = d.data;
      try {
        if (data === undefined || data === null) go(null, null);
        else if (typeof data === "string") go(toB64(new TextEncoder().encode(data)), null);
        else if (data instanceof ArrayBuffer) go(toB64(new U8(data)), null);
        else if (ArrayBuffer.isView(data)) go(toB64(new U8(data.buffer, data.byteOffset, data.byteLength)), null);
        else {
          // FormData、Blob、URLSearchParams：交給瀏覽器轉成位元組，同時得到正確的 Content-Type（含分隔字串）
          const rsp = new Response(data);
          const ct = rsp.headers.get("content-type");
          rsp.arrayBuffer().then((buf) => go(toB64(new U8(buf)), ct), () => finish("error"));
        }
      } catch (_) { fail("error"); }
      return { abort: self.abort };
    };

    const download = (a, b) => {
      const d = typeof a === "string" ? { url: a, name: b } : (a || {});
      return xhr({
        url: d.url, method: d.method || "GET", headers: d.headers, responseType: "blob", timeout: d.timeout,
        onload: (r) => {
          if (!(r.status >= 200 && r.status < 300)) { if (typeof d.onerror === "function") d.onerror({ error: "not_succeeded", details: "HTTP " + r.status }); return; }
          try {
            const u = URL.createObjectURL(r.response), el = D.createElement("a");
            el.href = u; el.download = String(d.name || "download"); el.style.display = "none";
            (D.body || D.documentElement).appendChild(el); el.click(); el.remove();
            setT(() => URL.revokeObjectURL(u), 60000);
            if (typeof d.onload === "function") d.onload({});
          } catch (e) { if (typeof d.onerror === "function") d.onerror({ error: "not_succeeded", details: String(e) }); }
        },
        onerror: () => { if (typeof d.onerror === "function") d.onerror({ error: "not_succeeded" }); },
        ontimeout: () => { if (typeof d.ontimeout === "function") d.ontimeout({ error: "timeout" }); },
        onprogress: d.onprogress,
      });
    };

    const openInTab = (u, o) => {
      const opt = typeof o === "boolean" ? { active: !o } : (o && typeof o === "object" ? o : {});
      const active = opt.active !== undefined ? !!opt.active : opt.loadInBackground !== undefined ? !opt.loadInBackground : true;
      let url;
      try { url = new URL(String(u), L.href).href; } catch (_) { return undefined; }
      if (!send({ op: "tab", s: s.id, url, active, insert: opt.insert !== false })) { try { W.open(url, "_blank"); } catch (_) {} }
      return { close: () => {}, closed: false, onclose: null };
    };
    const notification = (a, b, c) => {
      const d = typeof a === "string" ? { text: a, title: typeof b === "string" ? b : "" } : (a || {});
      send({ op: "notify", s: s.id, title: String(d.title || s.name), text: String(d.text || "") });
      const done = typeof c === "function" ? c : d.ondone;
      if (typeof done === "function") setT(() => { try { done(); } catch (_) {} }, 0);
    };
    const setClipboard = (t, _i, cb) => {
      if (!send({ op: "clip", s: s.id, text: String(t) })) { try { navigator.clipboard.writeText(String(t)); } catch (_) {} }
      if (typeof cb === "function") setT(() => { try { cb(); } catch (_) {} }, 0);
    };
    const log = (...a) => console.log(...a);
    // 這個分頁自己的小記事（重新整理就沒了）
    let tabObj = {};
    const getTab = (cb) => { if (typeof cb === "function") cb(tabObj); };
    const saveTab = (o, cb) => { tabObj = o && typeof o === "object" ? o : {}; if (typeof cb === "function") cb(); };
    const getTabs = (cb) => { if (typeof cb === "function") cb({ 0: tabObj }); };

    const P = (f) => (...a) => { try { return PromiseC.resolve(f(...a)); } catch (e) { return PromiseC.reject(e); } };
    const xhrP = (d) => {
      let ctl;
      const p = new PromiseC((res, rej) => { ctl = xhr(Object.assign({}, d, { onload: res, onerror: rej, ontimeout: rej, onabort: rej })); });
      p.abort = () => ctl && ctl.abort();
      return p;
    };
    // 腳本有寫 @grant 的才給（跟篡改猴一樣；很多腳本靠「有沒有這個功能」決定怎麼做）
    const G = (ok, f) => (ok ? f : undefined);
    const g = {
      getValue: can("GM_getValue", "GM.getValue"), setValue: can("GM_setValue", "GM.setValue"), deleteValue: can("GM_deleteValue", "GM.deleteValue"),
      listValues: can("GM_listValues", "GM.listValues"), getValues: can("GM_getValues", "GM.getValues"), setValues: can("GM_setValues", "GM.setValues"),
      deleteValues: can("GM_deleteValues", "GM.deleteValues"),
      listen: can("GM_addValueChangeListener", "GM.addValueChangeListener"), unlisten: can("GM_removeValueChangeListener", "GM.removeValueChangeListener", "GM_addValueChangeListener", "GM.addValueChangeListener"),
      addStyle: can("GM_addStyle", "GM.addStyle"), addElement: can("GM_addElement", "GM.addElement"),
      resText: can("GM_getResourceText", "GM.getResourceText"), resUrl: can("GM_getResourceURL", "GM.getResourceUrl", "GM.getResourceURL"),
      menu: can("GM_registerMenuCommand", "GM.registerMenuCommand"), unmenu: can("GM_unregisterMenuCommand", "GM.unregisterMenuCommand", "GM_registerMenuCommand", "GM.registerMenuCommand"),
      xhr: can("GM_xmlhttpRequest", "GM.xmlHttpRequest", "GM.xmlhttpRequest"), download: can("GM_download", "GM.download"),
      tab: can("GM_openInTab", "GM.openInTab"), notify: can("GM_notification", "GM.notification"), clip: can("GM_setClipboard", "GM.setClipboard"),
      log: can("GM_log", "GM.log"), tabs: can("GM_getTab", "GM_saveTab", "GM_getTabs", "GM.getTab", "GM.saveTab", "GM.getTabs"),
    };
    const GM = { info };
    const put = (ok, name, f) => { if (ok) GM[name] = f; };
    put(g.getValue, "getValue", P(getValue)); put(g.setValue, "setValue", P(setValue)); put(g.deleteValue, "deleteValue", P(deleteValue));
    put(g.listValues, "listValues", P(listValues)); put(g.getValues, "getValues", P(getValues)); put(g.setValues, "setValues", P(setValues));
    put(g.deleteValues, "deleteValues", P(deleteValues)); put(g.listen, "addValueChangeListener", P(addListener)); put(g.unlisten, "removeValueChangeListener", P(removeListener));
    put(g.addStyle, "addStyle", P(addStyle)); put(g.addElement, "addElement", P(addElement));
    put(g.resText, "getResourceText", P(resText)); put(g.resUrl, "getResourceUrl", P(resUrl)); put(g.resUrl, "getResourceURL", P(resUrl));
    put(g.menu, "registerMenuCommand", P(registerMenu)); put(g.unmenu, "unregisterMenuCommand", P(unregisterMenu));
    put(g.xhr, "xmlHttpRequest", xhrP); put(g.xhr, "xmlhttpRequest", xhrP); put(g.download, "download", P(download));
    put(g.tab, "openInTab", P(openInTab)); put(g.notify, "notification", P(notification)); put(g.clip, "setClipboard", P(setClipboard));
    put(g.log, "log", P(log)); put(g.tabs, "getTab", () => new PromiseC((r) => getTab(r))); put(g.tabs, "saveTab", P(saveTab)); put(g.tabs, "getTabs", () => new PromiseC((r) => getTabs(r)));
    return {
      remote,
      sync: (now) => {
        for (const k of keysOf(store)) if (typeof now[k] !== "string") remote(k, null);
        for (const k of keysOf(now)) if (typeof now[k] === "string") remote(k, now[k]);
      },
      menu: (id) => { const f = menus[String(id)]; if (f) { try { f(new MouseEvent("click")); } catch (e) { warn(s, e); } } },
      // 順序要跟程式那邊包腳本時寫的參數一樣
      args: [GM, info, G(g.getValue, getValue), G(g.setValue, setValue), G(g.deleteValue, deleteValue), G(g.listValues, listValues), G(g.getValues, getValues),
        G(g.setValues, setValues), G(g.deleteValues, deleteValues), G(g.listen, addListener), G(g.unlisten, removeListener), G(g.addStyle, addStyle),
        G(g.addElement, addElement), G(g.resText, resText), G(g.resUrl, resUrl), G(g.menu, registerMenu), G(g.unmenu, unregisterMenu), G(g.xhr, xhr),
        G(g.download, download), G(g.tab, openInTab), G(g.notify, notification), G(g.clip, setClipboard), G(g.log, log), G(g.tabs, getTab), G(g.tabs, saveTab),
        G(g.tabs, getTabs), W],
    };
  }

  /* ---------- 啟動：這份文件該跑哪些腳本 ---------- */
  function run(ver, getVals) {
    // 比對網址時不看 # 後面
    const href = L.href.replace(/#.*$/, "");
    const hit = (list) => list.some((r) => { try { return new RegExp(r).test(href); } catch (_) { return false; } });
    const mine = defs.filter((s) => (TOP || s.frames) && hit(s.inc) && !hit(s.exc));
    defs = [];
    // 最上層的網頁一定跟程式報到（就算沒有腳本要跑）：程式才知道這個分頁換了一頁，舊的選單要清掉
    if (TOP || mine.length) send({ op: "hi", ids: mine.map((s) => s.id), ver });
    if (!mine.length) return;
    const vals = getVals();
    // 「檢視網頁原始碼」的頁面不跑（網址跟原本的網頁一樣，要從內容認：最前面有一個行號的底）
    const source = () => { try { return !!D.querySelector("body > div.line-gutter-backdrop"); } catch (_) { return false; } };
    const exec = (s) => {
      if (s.ran) return;
      s.ran = true;
      try { apply(s.fn, W, s.api.args); } catch (e) { warn(s, e); }
    };
    for (const s of mine) {
      s.api = makeApi(s, vals[s.id] && typeof vals[s.id] === "object" ? vals[s.id] : {});
      apis[s.id] = s.api;
    }
    const at = (w) => { if (!source()) mine.filter((s) => s.at === w).forEach(exec); };
    at("start");
    // document-body：<body> 一出現；document-end：內容讀完；document-idle（預設）：再晚一點
    if (mine.some((s) => s.at === "body")) {
      if (D.body) at("body");
      else { const ob = new MutationObserver(() => { if (D.body) { ob.disconnect(); at("body"); } }); ob.observe(D, { childList: true, subtree: true }); }
    }
    const ready = () => { at("body"); at("end"); setT(() => at("idle"), 0); };
    if (D.readyState === "loading") D.addEventListener("DOMContentLoaded", ready, { once: true }); else ready();
  }
})();
