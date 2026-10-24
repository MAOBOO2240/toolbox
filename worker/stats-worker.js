// 工具箱使用狀況收集端（Cloudflare Worker）
// 綁定：KV 命名空間 STATS；環境變數 ADMIN_KEY（看儀表板用的密碼）
//
// POST /ping   程式回報：{ serial, install, version, run, os, licensed }
// GET  /admin?key=ADMIN_KEY            儀表板
// POST /note?key=ADMIN_KEY  { serial, note }  幫某個序號加備註（例如是誰）

const html = (s) => new Response(s, { headers: { "content-type": "text/html; charset=utf-8" } });
const json = (o, status = 200) => new Response(JSON.stringify(o), { status, headers: { "content-type": "application/json" } });
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (req.method === "POST" && url.pathname === "/ping") {
      let p;
      try { p = await req.json(); } catch { return json({ ok: false }, 400); }
      const serial = String(p.serial || "").slice(0, 32);
      if (!/^MB-[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}$/.test(serial)) return json({ ok: false }, 400);
      const key = "s:" + serial;
      const old = (await env.STATS.get(key, "json")) || {};
      const now = new Date().toISOString();
      const rec = {
        serial,
        install: String(p.install || "").slice(0, 32),
        version: String(p.version || "").slice(0, 12),
        run: String(p.run || "").slice(0, 12),
        os: String(p.os || "").slice(0, 40),
        licensed: !!p.licensed,
        first: old.first || now,
        last: now,
        pings: (old.pings || 0) + 1,
        note: old.note || "",
        ip_country: req.headers.get("cf-ipcountry") || "",
      };
      await env.STATS.put(key, JSON.stringify(rec));
      return json({ ok: true });
    }
    if (url.searchParams.get("key") !== env.ADMIN_KEY || !env.ADMIN_KEY) return new Response("Not found", { status: 404 });
    if (req.method === "POST" && url.pathname === "/note") {
      const p = await req.json().catch(() => ({}));
      const key = "s:" + String(p.serial || "");
      const rec = await env.STATS.get(key, "json");
      if (!rec) return json({ ok: false }, 404);
      rec.note = String(p.note || "").slice(0, 80);
      await env.STATS.put(key, JSON.stringify(rec));
      return json({ ok: true });
    }
    if (url.pathname === "/admin" || url.pathname === "/") {
      const list = [];
      let cursor;
      do {
        const r = await env.STATS.list({ prefix: "s:", cursor });
        for (const k of r.keys) { const v = await env.STATS.get(k.name, "json"); if (v) list.push(v); }
        cursor = r.list_complete ? undefined : r.cursor;
      } while (cursor);
      list.sort((a, b) => (b.last || "").localeCompare(a.last || ""));
      const day = 86400000, now = Date.now();
      const active = (ms) => list.filter((x) => now - Date.parse(x.last) < ms).length;
      const rows = list.map((x) => `<tr>
        <td><code>${esc(x.serial)}</code></td>
        <td><input value="${esc(x.note)}" data-serial="${esc(x.serial)}" placeholder="這是誰" /></td>
        <td>${x.licensed ? "✅ 已授權" : "—"}</td>
        <td>${esc(x.version)}${x.run ? " #" + esc(x.run) : ""}</td>
        <td>${esc(x.os)}</td>
        <td>${esc(x.ip_country)}</td>
        <td title="${esc(x.first)}">${esc(x.first.slice(0, 10))}</td>
        <td title="${esc(x.last)}">${esc(x.last.replace("T", " ").slice(0, 16))} UTC</td>
        <td>${x.pings}</td></tr>`).join("");
      return html(`<!doctype html><meta charset="utf-8"><title>工具箱使用狀況</title>
<style>body{font:14px system-ui,sans-serif;margin:2rem;background:#14161c;color:#eee}table{border-collapse:collapse;width:100%}th,td{padding:.4rem .6rem;border-bottom:1px solid #333;text-align:left;white-space:nowrap}th{color:#9aa}input{background:#222;color:#eee;border:1px solid #444;border-radius:4px;padding:.2rem .4rem;width:10rem}code{color:#7dd3fc}.sum{display:flex;gap:2rem;margin-bottom:1rem}.sum b{font-size:1.6rem;display:block}</style>
<h1>工具箱使用狀況</h1>
<div class="sum"><div>總安裝數<b>${list.length}</b></div><div>今天有開<b>${active(day)}</b></div><div>7 天內<b>${active(7 * day)}</b></div><div>30 天內<b>${active(30 * day)}</b></div><div>已授權<b>${list.filter((x) => x.licensed).length}</b></div></div>
<table><tr><th>序號</th><th>備註</th><th>授權</th><th>版本</th><th>系統</th><th>地區</th><th>第一次</th><th>最後一次</th><th>回報次數</th></tr>${rows}</table>
<p style="color:#9aa">備註改完會自動儲存。時間是 UTC（台灣 +8 小時）。程式每 6 小時回報一次，只送序號、版本、Windows 版本與有沒有授權。</p>
<script>document.querySelectorAll("input[data-serial]").forEach((i)=>i.addEventListener("change",async()=>{await fetch("/note?key="+encodeURIComponent(new URLSearchParams(location.search).get("key")),{method:"POST",body:JSON.stringify({serial:i.dataset.serial,note:i.value})});i.style.borderColor="#4c4";}));</script>`);
    }
    return new Response("Not found", { status: 404 });
  },
};
