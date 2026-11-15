(async () => {
  // 這段程式在「篡改猴套件裡的 manifest.json」那份文件裡執行（那只是一份文字檔，不會執行篡改猴的任何程式），
  // 用擴充功能自己的存放區把使用者的腳本讀出來，結果是一段 JSON 文字。
  // known = 程式那邊已經有的 { 編號: [內容的雜湊, 設定值的時間] }：一樣的就不傳內容
  const known = __KNOWN__;
  const out = (o) => JSON.stringify(o);
  const L = typeof chrome === "object" && chrome && chrome.storage && chrome.storage.local;
  if (!L) return out({ ok: false, why: "no-storage" });
  try {
    // 篡改猴存的每一筆都包成 { origin, value }
    const un = (v) => (v && typeof v === "object" && !Array.isArray(v) && "value" in v ? v.value : v);
    const isSrc = (v) => typeof v === "string" && v.includes("==UserScript==") && v.includes("==/UserScript==");
    let keys = null;
    try {
      if (typeof L.getKeys === "function") keys = await L.getKeys();
    } catch (e) {
      keys = null;
    }
    // 認得的存法：只拿腳本的內容、資訊、設定值那幾種；不認得（篡改猴改了存法）：全部拿來，靠內容認
    const usual = Array.isArray(keys) && keys.some((k) => k.includes("@source#"));
    window.__mbTmUsual = usual;
    const data = usual ? await L.get(keys.filter((k) => /@(source|meta|st)#/.test(k))) : await L.get(null);
    // 同一支腳本的幾筆資料，鍵的結尾（# 後面）是同一個編號
    const by = new Map();
    const at = (k) => {
      const i = k.lastIndexOf("#");
      const id = i < 0 ? k : k.slice(i + 1);
      let e = by.get(id);
      if (!e) by.set(id, (e = {}));
      return e;
    };
    for (const k of Object.keys(data)) {
      const v = un(data[k]);
      if (typeof v === "string") {
        if (isSrc(v) && (k.includes("@source#") || !at(k).src)) at(k).src = v;
      } else if (v && typeof v === "object" && !Array.isArray(v)) {
        if (typeof v.enabled === "boolean") at(k).meta = v;
        else if (v.data && typeof v.data === "object" && !Array.isArray(v.data) && "ts" in v) at(k).st = v;
      }
    }
    const strs = (a) => (Array.isArray(a) && a.every((x) => typeof x === "string") ? a : null);
    const hex = async (s) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s))), (b) => b.toString(16).padStart(2, "0")).join("");
    const list = [];
    for (const [id, e] of by) {
      if (!e.src) continue;
      const m = e.meta || {};
      const o = m.options && typeof m.options === "object" ? m.options : {};
      const hash = await hex(e.src);
      const k = Array.isArray(known[id]) ? known[id] : [];
      const ts = e.st && typeof e.st.ts === "number" && e.st.ts > 0 && e.st.ts < 9e15 ? Math.floor(e.st.ts) : 0;
      const row = {
        id,
        hash,
        enabled: m.enabled !== false,
        ts,
        // 篡改猴把腳本自己寫的網址和使用者另外加的合起來之後的結果；什麼時候執行、只在最上層執行是使用者另外設定的
        opts: {
          includes: strs(m.includes),
          matches: strs(m.matches),
          excludes: strs(m.excludes),
          run_at: typeof o.run_at === "string" ? o.run_at : null,
          noframes: typeof o.noframes === "boolean" ? o.noframes : null,
        },
      };
      if (k[0] !== hash) row.src = e.src;
      if (ts && k[1] !== ts && Object.keys(e.st.data).length) row.vals = e.st.data;
      list.push(row);
    }
    return out({ ok: true, list });
  } catch (e) {
    return out({ ok: false, why: "error: " + ((e && e.message) || e) });
  }
})()
