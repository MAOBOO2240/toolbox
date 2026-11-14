(() => {
  // 使用者腳本的執行環境（每份文件一開始就跑；內嵌框架也有）。
  // 這一段只負責準備：腳本本身由後面各自登記的小段程式交進來（add），最後由「帶最新設定值」的那一段啟動（go）。
  const W = window, D = document;
  if (location.origin === "__APP_ORIGIN__") return;
  const TOKEN = "__TOKEN__";
  const TOP = W.top === W;

  // 先把會用到的內建功能抓好：之後網頁就算把它們換掉，也影響不到跟程式之間的訊息
  const jparse = JSON.parse, jstr = JSON.stringify;
  const keysOf = Object.keys, defProp = Object.defineProperty;
  const setT = W.setTimeout.bind(W), clearT = W.clearTimeout.bind(W);
  const U8 = W.Uint8Array, PromiseC = W.Promise;
  const b64dec = W.atob.bind(W), b64enc = W.btoa.bind(W);
  const fromChar = String.fromCharCode;

  /* ---------- 跟程式互傳訊息的通道 ---------- */
  const C = W.chrome;
  const wv = C && C.webview;
  let post = null;
  if (wv && typeof wv.postMessage === "function") {
    post = wv.postMessage.bind(wv);
    wv.addEventListener("message", (e) => {
      const d = e && e.data;
      if (d && d.mb === TOKEN) onApp(d);
    });
    // 網頁自己的程式不該碰得到這個通道（不能假裝成腳本傳訊息給程式，也看不到程式回的內容）
    try { delete C.webview; } catch (_) {}
    if (C.webview) { try { defProp(C, "webview", { value: undefined, configurable: true, writable: true }); } catch (_) {} }
  }
  const send = (m) => {
    if (!post) return false;
    m.mb = TOKEN;
    try { post(m); return true; } catch (_) { return false; }
  };

  /* ---------- 腳本交進來、啟動 ---------- */
  const defs = [];
  let baked = null, started = false;
  const start = (vals) => {
    if (started) return;
    started = true;
    try { delete W.__mbus; } catch (_) {}
    try { run(vals || baked || {}); } catch (e) { console.error("[使用者腳本] 啟動失敗", e); }
  };
  try {
    defProp(W, "__mbus", {
      configurable: true,
      value: {
        add: (tok, d) => { if (tok === TOKEN && !started && d && typeof d.fn === "function") defs.push(d); },
        baked: (tok, v) => { if (tok === TOKEN) baked = v; },
        go: (tok, v) => { if (tok === TOKEN) start(v); },
      },
    });
  } catch (_) { return; }
  // 備用：分頁剛建立的那一下，「帶最新設定值啟動」的那一段可能還沒登記好。網頁出現第一個元素時
  // （一定在網頁自己的程式之前）還沒啟動，就用建立分頁時的設定值啟動
  const fallback = () => start(null);
  try { new MutationObserver((_, ob) => { ob.disconnect(); fallback(); }).observe(D, { childList: true, subtree: true }); } catch (_) {}
  D.addEventListener("DOMContentLoaded", fallback, { once: true });

  /* ---------- 程式傳來的訊息 ---------- */
  const apis = new Map();   // 腳本編號 → 這份文件裡那支腳本的狀態
  const xhrs = new Map();   // 連線編號 → 進行中的 GM_xmlhttpRequest
  let seq = 0;
  function onApp(d) {
    if (d.ev === "val") {
      const a = apis.get(d.s);
      if (a) a.remote(d.k, d.v);
    } else if (d.ev === "menu") {
      const a = apis.get(d.s);
      if (a) a.menu(d.id);
    } else if (d.ev === "xhr") {
      const x = xhrs.get(d.rid);
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

  /* ---------- 一支腳本的 GM_* 功能 ---------- */
  function makeApi(s, store) {
    const info = {
      script: {
        name: s.name, version: s.version, description: s.desc, namespace: s.ns, author: s.author, homepage: s.home,
        matches: s.matches, includes: s.includes, excludes: s.excludes, grant: s.grants, connects: s.conn,
        "run-at": s.at === "start" ? "document-start" : s.at === "body" ? "document-body" : s.at === "end" ? "document-end" : "document-idle",
        resources: keysOf(s.res || {}).map((n) => ({ name: n })), updateURL: s.upd, downloadURL: s.dl, unwrap: false,
      },
      scriptHandler: "MAOBOO", version: "2.0", scriptWillUpdate: !!(s.upd || s.dl), isIncognito: false,
    };

    // ---- 設定值：整個瀏覽器共用（不分網站），別的分頁改了這裡也會跟著變 ----
    const listeners = [];
    let lseq = 0;
    const val = (j, d) => { if (j === undefined || j === null) return d; try { return jparse(j); } catch (_) { return d; } };
    const fire = (k, oj, nj, remote) => {
      for (const l of listeners.slice()) {
        if (l.k !== k) continue;
        try { l.fn(k, val(oj, undefined), val(nj, undefined), remote); } catch (e) { console.error("[使用者腳本] " + s.name, e); }
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
    const menus = new Map();
    let mseq = 0;
    const registerMenu = (caption, fn, o) => {
      if (!TOP) return undefined;
      const opt = typeof o === "string" ? { accessKey: o } : (o && typeof o === "object" ? o : {});
      const id = opt.id !== undefined && opt.id !== null ? String(opt.id) : "m" + (++mseq);
      menus.set(id, typeof fn === "function" ? fn : () => {});
      send({ op: "menu", s: s.id, id, caption: String(caption), title: opt.title ? String(opt.title) : "", close: opt.autoClose !== false });
      return id;
    };
    const unregisterMenu = (id) => { id = String(id); if (menus.delete(id)) send({ op: "unmenu", s: s.id, id }); };

    // ---- 連到別的網站（由程式代為連線，不受網頁的跨網站限制）----
    const xhr = (d) => {
      d = d || {};
      const rid = ++seq;
      const st = { done: false, chunks: [], loaded: 0, total: 0, head: null, stream: null, ctl: null };
      const call = (name, r) => { const f = d[name]; if (typeof f === "function") { try { f(r); } catch (e) { console.error("[使用者腳本] " + s.name, e); } } };
      const rt = String(d.responseType || "").toLowerCase();
      const base = (ready) => {
        const h = st.head || {};
        return { finalUrl: h.url || "", readyState: ready, status: h.status || 0, statusText: h.statusText || "", responseHeaders: h.headers || "", context: d.context };
      };
      const finish = (kind) => {
        if (st.done) return;
        st.done = true;
        xhrs.delete(rid);
        if (st.ctl) { try { kind === "load" ? st.ctl.close() : st.ctl.error(new Error(kind)); } catch (_) {} }
        if (kind === "load") {
          const r = base(4);
          const bytes = concat(st.chunks, st.loaded);
          const ct = d.overrideMimeType || (/^content-type:\s*(.*)$/im.exec(r.responseHeaders) || [])[1] || "";
          if (rt === "arraybuffer") r.response = bytes.buffer;
          else if (rt === "blob") r.response = new Blob([bytes], { type: ct.split(";")[0].trim() });
          else if (rt === "stream") r.response = st.stream;
          else {
            const text = decode(bytes, charsetOf(ct));
            r.responseText = text;
            if (rt === "json") { try { r.response = jparse(text); } catch (_) { r.response = null; } }
            else if (rt === "document") { try { r.response = new DOMParser().parseFromString(text, /xml/i.test(ct) && !/html/i.test(ct) ? "text/xml" : "text/html"); r.responseXML = r.response; } catch (_) { r.response = null; } }
            else r.response = text;
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
            call("onprogress", r);
          } else if (m.t === "end") finish("load");
          else if (m.t === "err") finish(m.kind === "timeout" ? "timeout" : m.kind === "abort" ? "abort" : "error");
        },
        abort: () => { if (!st.done) { send({ op: "xhr-abort", rid }); finish("abort"); } },
      };
      let url;
      try { url = new URL(String(d.url), location.href).href; } catch (_) { fail("error"); return { abort: () => {} }; }
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
      xhrs.set(rid, self);
      const headers = {};
      if (d.headers && typeof d.headers === "object") for (const k of keysOf(d.headers)) headers[String(k)] = String(d.headers[k]);
      const go = (body, ctype) => {
        if (st.done) return;
        if (ctype && !keysOf(headers).some((k) => k.toLowerCase() === "content-type")) headers["Content-Type"] = ctype;
        const ok = send({
          op: "xhr", s: s.id, rid, url, method: String(d.method || "GET").toUpperCase(), headers, body,
          timeout: +d.timeout > 0 ? +d.timeout : 0, anon: !!d.anonymous, ua: navigator.userAgent, ref: location.href,
          user: d.user !== undefined && d.user !== null ? String(d.user) : undefined, password: d.password !== undefined && d.password !== null ? String(d.password) : undefined,
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
      try { url = new URL(String(u), location.href).href; } catch (_) { return undefined; }
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
    const P = (f) => (...a) => { try { return PromiseC.resolve(f(...a)); } catch (e) { return PromiseC.reject(e); } };
    const xhrP = (d) => {
      let ctl;
      const p = new PromiseC((res, rej) => { ctl = xhr(Object.assign({}, d, { onload: res, onerror: rej, ontimeout: rej, onabort: rej })); });
      p.abort = () => ctl && ctl.abort();
      return p;
    };
    const GM = {
      info: info, getValue: P(getValue), setValue: P(setValue), deleteValue: P(deleteValue), listValues: P(listValues),
      getValues: P(getValues), setValues: P(setValues), deleteValues: P(deleteValues),
      addValueChangeListener: P(addListener), removeValueChangeListener: P(removeListener),
      addStyle: P(addStyle), addElement: P(addElement), getResourceText: P(resText), getResourceUrl: P(resUrl), getResourceURL: P(resUrl),
      registerMenuCommand: P(registerMenu), unregisterMenuCommand: P(unregisterMenu), xmlHttpRequest: xhrP, download: P(download),
      openInTab: P(openInTab), notification: P(notification), setClipboard: P(setClipboard), log: P(log),
    };
    return {
      remote: (k, j) => {
        const old = store[k];
        if (j === null || j === undefined) { if (old === undefined) return; delete store[k]; fire(k, old, undefined, true); }
        else { if (old === j) return; store[k] = j; fire(k, old, j, true); }
      },
      menu: (id) => { const f = menus.get(String(id)); if (f) { try { f(new MouseEvent("click")); } catch (e) { console.error("[使用者腳本] " + s.name, e); } } },
      args: [GM, info, getValue, setValue, deleteValue, listValues, getValues, setValues, deleteValues, addListener, removeListener, addStyle, addElement,
        resText, resUrl, registerMenu, unregisterMenu, xhr, download, openInTab, notification, setClipboard, log, W],
    };
  }

  /* ---------- 啟動：這份文件該跑哪些腳本 ---------- */
  function run(vals) {
    // 「檢視網頁原始碼」的頁面不跑（網址跟原本的網頁一樣，要從內容認：最前面有一個行號的底）
    if (D.querySelector("body > div.line-gutter-backdrop")) return;
    const href = location.href;
    const hit = (list) => list.some((r) => { try { return new RegExp(r).test(href); } catch (_) { return false; } });
    const mine = defs.filter((s) => (TOP || s.frames) && hit(s.inc) && !hit(s.exc));
    if (!mine.length) return;
    const exec = (s) => {
      if (s.ran) return;
      s.ran = true;
      try { s.fn.apply(W, s.api.args); } catch (e) { console.error("[使用者腳本] " + s.name, e); }
    };
    for (const s of mine) {
      s.api = makeApi(s, vals[s.id] && typeof vals[s.id] === "object" ? Object.assign({}, vals[s.id]) : {});
      apis.set(s.id, s.api);
    }
    for (const s of mine) if (s.at === "start") exec(s);
    const at = (w) => mine.filter((s) => s.at === w).forEach(exec);
    // document-body：<body> 一出現；document-end：內容讀完；document-idle（預設）：再晚一點
    if (mine.some((s) => s.at === "body")) {
      if (D.body) at("body");
      else { const ob = new MutationObserver(() => { if (D.body) { ob.disconnect(); at("body"); } }); ob.observe(D, { childList: true, subtree: true }); }
    }
    const ready = () => { at("body"); at("end"); setT(() => at("idle"), 0); };
    if (D.readyState === "loading") D.addEventListener("DOMContentLoaded", ready, { once: true }); else ready();
  }
})();
