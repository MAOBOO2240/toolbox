// 工具箱使用狀況收集端 + 公會自動授權（Cloudflare Worker）
// 綁定：KV 命名空間 STATS；Secret：ADMIN_KEY（看儀表板用的密碼）、NEXON_API_KEY（查公會用）
//
// POST /ping   程式回報：{ serial, install, version, run, os, licensed, name }
// POST /auto   公會自動授權：{ serial, name } → { ok, code, name, world, guild } 或 { ok:false, msg }
// 以下都要 ?key=ADMIN_KEY
// GET  /admin                        儀表板
// POST /note    { serial, note }     幫某個序號加備註
// POST /cfg     { guilds }           允許的公會（每行「伺服器 公會」）
// POST /keygen                       產生自動授權用的金鑰（只會產生一次）
// POST /unbind  { name, serial }     解除某個角色跟某台電腦的綁定
// POST /recheck                      重新查所有綁定角色的公會
//
// KV 內容：s:<序號> 每台電腦；c:<角色> 綁定角色；r:<時間> 被拒絕的紀錄；cfg:guilds / cfg:sk / cfg:pk

const NEXON = "https://open.api.nexon.com/maplestorytw/v1";
const MAX_PC = 3; // 一個角色最多綁幾台電腦
const DEFAULT_GUILDS = [{ world: "艾麗雅", guild: "楓林小築" }];

const html = (s) => new Response(s, { headers: { "content-type": "text/html; charset=utf-8" } });
const json = (o, status = 200) => new Response(JSON.stringify(o), { status, headers: { "content-type": "application/json; charset=utf-8" } });
const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const SERIAL_RE = /^MB-[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}$/;
const hex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
const b64url = (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const tw = (iso) => { if (!iso) return "—"; const d = new Date(Date.parse(iso) + 8 * 3600000); return d.toISOString().replace("T", " ").slice(0, 16); };

async function listAll(env, prefix) {
  const out = [];
  let cursor;
  do {
    const r = await env.STATS.list({ prefix, cursor });
    for (const k of r.keys) { const v = await env.STATS.get(k.name, "json"); if (v) out.push(v); }
    cursor = r.list_complete ? undefined : r.cursor;
  } while (cursor);
  return out;
}

async function guilds(env) {
  return (await env.STATS.get("cfg:guilds", "json")) || DEFAULT_GUILDS;
}
const allowed = (gs, c) => !!c && gs.some((g) => g.world === c.world && g.guild === c.guild);

/* ---------- Nexon API：查角色的伺服器與公會 ---------- */
async function nexon(env, path, params) {
  const u = new URL(NEXON + path);
  for (const [k, v] of Object.entries(params)) u.searchParams.set(k, v);
  const r = await fetch(u, { headers: { "x-nxopen-api-key": env.NEXON_API_KEY } });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) {
    const name = j?.error?.name || "";
    if (r.status === 400 && name === "OPENAPI00004") throw new Error("找不到這個角色（名稱要跟遊戲裡完全一樣；新角色或剛改名要等隔天）");
    if (name === "OPENAPI00007") throw new Error("今天的 API 次數已用完，明天再試");
    if (name === "OPENAPI00005" || name === "OPENAPI00006") throw new Error("作者的 API 金鑰無效或已過期");
    throw new Error("Nexon API 錯誤 " + r.status + " " + name);
  }
  return j;
}

/** 回傳 { name, world, guild, ocid, level, class } */
async function lookup(env, name) {
  if (!env.NEXON_API_KEY) throw new Error("作者還沒在 Worker 設定 NEXON_API_KEY");
  const { ocid } = await nexon(env, "/id", { character_name: name });
  if (!ocid) throw new Error("找不到這個角色");
  const b = await nexon(env, "/character/basic", { ocid });
  return { name: b.character_name || name, world: b.world_name || "", guild: b.character_guild_name || "", ocid, level: b.character_level || 0, class: b.character_class || "" };
}

/** 重新查一個綁定角色的公會並存回去 */
async function refresh(env, c) {
  try {
    const info = await lookup(env, c.name);
    Object.assign(c, { world: info.world, guild: info.guild, level: info.level, class: info.class, ocid: info.ocid, checked: new Date().toISOString(), err: "" });
  } catch (e) {
    c.checked = new Date().toISOString();
    c.err = String(e.message || e);
  }
  await env.STATS.put("c:" + c.name, JSON.stringify(c));
  return c;
}

async function reject(env, o) {
  // key 用「倒過來的時間」讓最新的排最前面；90 天後自動刪
  const k = "r:" + String(9999999999999 - Date.now()).padStart(13, "0") + ":" + Math.random().toString(36).slice(2, 6);
  await env.STATS.put(k, JSON.stringify({ ...o, ts: new Date().toISOString() }), { expirationTtl: 90 * 86400 });
}

/* ---------- Ed25519 簽名（跟程式裡的 license.rs 相同格式：MB1- + base64url(簽名)） ---------- */
async function signCode(env, serial) {
  const jwk = await env.STATS.get("cfg:sk", "json");
  if (!jwk) throw new Error("作者還沒在儀表板產生自動授權金鑰");
  const key = await crypto.subtle.importKey("jwk", jwk, { name: "Ed25519" }, false, ["sign"]);
  const sig = await crypto.subtle.sign({ name: "Ed25519" }, key, new TextEncoder().encode(serial));
  return "MB1-" + b64url(sig);
}

async function keygen(env) {
  if (await env.STATS.get("cfg:pk")) throw new Error("已經有金鑰了");
  const kp = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const jwk = await crypto.subtle.exportKey("jwk", kp.privateKey);
  const pk = hex(await crypto.subtle.exportKey("raw", kp.publicKey));
  await env.STATS.put("cfg:sk", JSON.stringify(jwk));
  await env.STATS.put("cfg:pk", pk);
  return pk;
}

/* ---------- 公會自動授權 ---------- */
async function auto(env, p) {
  const serial = String(p.serial || "").trim().toUpperCase();
  const name = String(p.name || "").trim();
  if (!SERIAL_RE.test(serial)) throw new Error("序號格式不對");
  if (!name || name.length > 20) throw new Error("請輸入角色名稱");
  if (!(await env.STATS.get("cfg:pk"))) throw new Error("作者還沒在儀表板產生自動授權金鑰");
  // 每台電腦一天最多試 20 次，省 API 次數
  const day = new Date().toISOString().slice(0, 10);
  const ak = `a:${serial}:${day}`;
  const tries = Number(await env.STATS.get(ak)) || 0;
  if (tries >= 20) throw new Error("今天嘗試太多次了，明天再試");
  await env.STATS.put(ak, String(tries + 1), { expirationTtl: 2 * 86400 });

  let info;
  try { info = await lookup(env, name); } catch (e) {
    await reject(env, { serial, name, world: "", guild: "", reason: "查不到：" + (e.message || e) });
    throw e;
  }
  const gs = await guilds(env);
  if (!allowed(gs, info)) {
    await reject(env, { serial, name: info.name, world: info.world, guild: info.guild, reason: "公會不符" });
    throw new Error(`角色「${info.name}」（${info.world}）的公會是「${info.guild || "無"}」，不在允許名單裡`);
  }
  const ck = "c:" + info.name;
  const c = (await env.STATS.get(ck, "json")) || { name: info.name, serials: [], first: new Date().toISOString() };
  if (!c.serials.includes(serial)) {
    if (c.serials.length >= MAX_PC) {
      await reject(env, { serial, name: info.name, world: info.world, guild: info.guild, reason: `這個角色已經綁了 ${MAX_PC} 台電腦` });
      throw new Error(`角色「${info.name}」已經在 ${MAX_PC} 台電腦上授權過了，請找作者解除`);
    }
    c.serials.push(serial);
  }
  Object.assign(c, { world: info.world, guild: info.guild, level: info.level, class: info.class, ocid: info.ocid, checked: new Date().toISOString(), err: "" });
  await env.STATS.put(ck, JSON.stringify(c));
  const code = await signCode(env, serial);
  const sk = "s:" + serial;
  const rec = (await env.STATS.get(sk, "json")) || { serial, first: new Date().toISOString(), last: new Date().toISOString(), pings: 0, note: "" };
  Object.assign(rec, { char: info.name, licensed: true, auto: true });
  await env.STATS.put(sk, JSON.stringify(rec));
  return { ok: true, code, name: info.name, world: info.world, guild: info.guild };
}

/* ---------- 儀表板 ---------- */
async function dashboard(env) {
  const [list, chars, rejects, gs] = await Promise.all([listAll(env, "s:"), listAll(env, "c:"), listAll(env, "r:"), guilds(env)]);
  const pk = (await env.STATS.get("cfg:pk")) || "";
  const cmap = Object.fromEntries(chars.map((c) => [c.name, c]));
  list.sort((a, b) => (b.last || "").localeCompare(a.last || ""));
  const day = 86400000, now = Date.now();
  const active = (ms) => list.filter((x) => now - Date.parse(x.last) < ms).length;
  const bad = chars.filter((c) => !allowed(gs, c));
  const rows = list.map((x) => {
    const c = x.char ? cmap[x.char] : null;
    const ok = c ? allowed(gs, c) : null;
    return `<tr>
      <td><code>${esc(x.serial)}</code></td>
      <td><input value="${esc(x.note)}" data-serial="${esc(x.serial)}" placeholder="這是誰" /></td>
      <td>${x.licensed ? (x.auto ? "✅ 自動" : "✅ 手動") : "—"}</td>
      <td>${x.char ? `<b>${esc(x.char)}</b>${c ? ` <small>${esc(c.world)} Lv.${esc(c.level)} ${esc(c.class)}</small>` : ""}` : "—"}</td>
      <td class="${ok === false ? "bad" : ok ? "good" : ""}">${c ? esc(c.guild || "無") : "—"}${c?.err ? ` <small title="${esc(c.err)}">⚠</small>` : ""}</td>
      <td title="${esc(x.first)}">${tw(x.first)}</td>
      <td title="${esc(x.last)}">${tw(x.last)}</td>
      <td>${esc(x.version)}${x.run ? " #" + esc(x.run) : ""}</td>
      <td>${esc(x.os)}</td>
      <td>${x.char ? `<button data-unbind="${esc(x.char)}" data-serial="${esc(x.serial)}">解除綁定</button>` : ""}</td></tr>`;
  }).join("");
  const badRows = bad.map((c) => `<tr><td><b>${esc(c.name)}</b></td><td>${esc(c.world)}</td><td class="bad">${esc(c.guild || "無")}</td><td>${c.serials.map((s) => `<code>${esc(s)}</code>`).join(" ")}</td><td title="${esc(c.checked)}">${tw(c.checked)}</td><td>${c.serials.map((s) => `<button data-unbind="${esc(c.name)}" data-serial="${esc(s)}">解除 ${esc(s.slice(-4))}</button>`).join(" ")}</td></tr>`).join("");
  const rejRows = rejects.slice(0, 200).map((r) => `<tr><td title="${esc(r.ts)}">${tw(r.ts)}</td><td><code>${esc(r.serial)}</code></td><td>${esc(r.name)}</td><td>${esc(r.world)}</td><td class="${r.reason === "公會不符" ? "bad" : ""}">${esc(r.guild || (r.world ? "無" : ""))}</td><td>${esc(r.reason)}</td></tr>`).join("");
  const gtext = gs.map((g) => `${g.world} ${g.guild}`).join("\n");
  return html(`<!doctype html><meta charset="utf-8"><title>工具箱使用狀況</title>
<style>body{font:14px system-ui,sans-serif;margin:2rem;background:#14161c;color:#eee}table{border-collapse:collapse;width:100%}th,td{padding:.4rem .6rem;border-bottom:1px solid #333;text-align:left;white-space:nowrap}th{color:#9aa}input,textarea{background:#222;color:#eee;border:1px solid #444;border-radius:4px;padding:.2rem .4rem;font:inherit}input{width:10rem}textarea{width:20rem;height:5rem}code{color:#7dd3fc}.sum{display:flex;gap:2rem;margin-bottom:1rem}.sum b{font-size:1.6rem;display:block}.bad{color:#f87171}.good{color:#4ade80}button{background:#2a2d36;color:#eee;border:1px solid #555;border-radius:4px;padding:.2rem .6rem;cursor:pointer}button:hover{background:#3a3d48}h2{font-size:1.1rem;margin:1.6rem 0 .5rem}.cfg{display:flex;gap:2rem;flex-wrap:wrap;align-items:flex-start;background:#1b1e26;padding:1rem;border-radius:8px}.cfg small{color:#9aa;display:block;margin-top:.3rem}.pk{font-family:monospace;word-break:break-all;color:#7dd3fc;max-width:40rem}</style>
<h1>工具箱使用狀況</h1>
<div class="sum"><div>總安裝數<b>${list.length}</b></div><div>今天有開<b>${active(day)}</b></div><div>7 天內<b>${active(7 * day)}</b></div><div>30 天內<b>${active(30 * day)}</b></div><div>已授權<b>${list.filter((x) => x.licensed).length}</b></div><div>綁定角色<b>${chars.length}</b></div><div>公會不符<b class="${bad.length ? "bad" : ""}">${bad.length}</b></div></div>
<div class="cfg">
  <div><div>允許自動授權的公會（每行「伺服器 公會名稱」）</div><textarea id="guilds">${esc(gtext)}</textarea><br><button id="saveG">儲存公會名單</button> <button id="recheck">重新查所有人的公會</button><small>改完名單後，已經綁定的人不會失效（授權碼是永久的），只會在下面「公會不符」列出來。</small></div>
  <div><div>自動授權金鑰（公鑰）</div>${pk ? `<div class="pk">${esc(pk)}</div><small>把這串公鑰交給開發者放進程式（PUBKEY2_HEX）。只需要做一次。</small>` : `<button id="keygen">產生金鑰</button><small>產生後把公鑰交給開發者放進程式，自動授權才會生效。</small>`}</div>
</div>
<h2>所有電腦</h2>
<table><tr><th>序號</th><th>備註（這是誰）</th><th>授權</th><th>綁定角色</th><th>公會</th><th>第一次使用</th><th>最後一次使用</th><th>版本</th><th>系統</th><th></th></tr>${rows}</table>
<h2>公會不符（已綁定但現在的公會不在名單）${bad.length ? "" : "：沒有"}</h2>
${bad.length ? `<table><tr><th>角色</th><th>伺服器</th><th>目前公會</th><th>綁定的電腦</th><th>上次查詢</th><th></th></tr>${badRows}</table>` : ""}
<h2>被拒絕的自動授權（最近 90 天）${rejects.length ? "" : "：沒有"}</h2>
${rejects.length ? `<table><tr><th>時間</th><th>序號</th><th>輸入的角色</th><th>伺服器</th><th>公會</th><th>原因</th></tr>${rejRows}</table>` : ""}
<p style="color:#9aa">備註改完會自動儲存。時間是台灣時間。程式每 6 小時回報一次：序號、版本、Windows 版本、有沒有授權、綁定的角色；綁定角色的公會每天重查一次。一個角色最多綁 ${MAX_PC} 台電腦；解除綁定只是放出名額，對方電腦上已經存的授權碼不會失效。</p>
<script>
const K="?key="+encodeURIComponent(new URLSearchParams(location.search).get("key"));
const post=async(p,b)=>{const r=await fetch(p+K,{method:"POST",body:JSON.stringify(b||{})});const j=await r.json().catch(()=>({}));if(!j.ok)alert(j.msg||"失敗");return j;};
document.querySelectorAll("input[data-serial]").forEach((i)=>i.addEventListener("change",async()=>{await post("/note",{serial:i.dataset.serial,note:i.value});i.style.borderColor="#4c4";}));
document.querySelectorAll("button[data-unbind]").forEach((b)=>b.addEventListener("click",async()=>{if(!confirm("解除「"+b.dataset.unbind+"」跟 "+b.dataset.serial+" 的綁定？"))return;await post("/unbind",{name:b.dataset.unbind,serial:b.dataset.serial});location.reload();}));
document.getElementById("saveG").onclick=async()=>{await post("/cfg",{guilds:document.getElementById("guilds").value});location.reload();};
document.getElementById("recheck").onclick=async()=>{const b=document.getElementById("recheck");b.disabled=true;b.textContent="查詢中…";const j=await post("/recheck");alert("已重查 "+(j.n||0)+" 個角色");location.reload();};
const kg=document.getElementById("keygen");if(kg)kg.onclick=async()=>{if(!confirm("產生自動授權金鑰？只能產生一次。"))return;await post("/keygen");location.reload();};
</script>`);
}

export default {
  async fetch(req, env, ctx) {
    const url = new URL(req.url);
    if (req.method === "POST" && url.pathname === "/ping") {
      let p;
      try { p = await req.json(); } catch { return json({ ok: false }, 400); }
      const serial = String(p.serial || "").slice(0, 32);
      if (!SERIAL_RE.test(serial)) return json({ ok: false }, 400);
      const key = "s:" + serial;
      const old = (await env.STATS.get(key, "json")) || {};
      const now = new Date().toISOString();
      // 程式送來的綁定角色：只有真的綁在這台電腦上才算（解除綁定後就不顯示）
      let ch = String(p.name || "").slice(0, 20);
      if (ch) {
        const c = await env.STATS.get("c:" + ch, "json");
        if (!c || !c.serials.includes(serial)) ch = "";
        else if (Date.now() - Date.parse(c.checked || 0) > 86400000) ctx.waitUntil(refresh(env, c)); // 公會一天重查一次
      }
      const rec = {
        serial,
        install: String(p.install || "").slice(0, 32),
        version: String(p.version || "").slice(0, 12),
        run: String(p.run || "").slice(0, 12),
        os: String(p.os || "").slice(0, 40),
        licensed: !!p.licensed,
        auto: !!old.auto,
        char: ch || (p.name === undefined ? old.char || "" : ""),
        first: old.first || now,
        last: now,
        pings: (old.pings || 0) + 1,
        note: old.note || "",
        ip_country: req.headers.get("cf-ipcountry") || "",
      };
      await env.STATS.put(key, JSON.stringify(rec));
      return json({ ok: true });
    }
    if (req.method === "POST" && url.pathname === "/auto") {
      const p = await req.json().catch(() => ({}));
      try { return json(await auto(env, p)); } catch (e) { return json({ ok: false, msg: String(e.message || e) }); }
    }
    if (url.searchParams.get("key") !== env.ADMIN_KEY || !env.ADMIN_KEY) return new Response("Not found", { status: 404 });
    if (req.method === "POST") {
      const p = await req.json().catch(() => ({}));
      try {
        if (url.pathname === "/note") {
          const key = "s:" + String(p.serial || "");
          const rec = await env.STATS.get(key, "json");
          if (!rec) return json({ ok: false, msg: "找不到這個序號" }, 404);
          rec.note = String(p.note || "").slice(0, 80);
          await env.STATS.put(key, JSON.stringify(rec));
          return json({ ok: true });
        }
        if (url.pathname === "/cfg") {
          const gs = String(p.guilds || "").split(/\r?\n/).map((l) => l.trim().split(/\s+/)).filter((a) => a.length >= 2).map((a) => ({ world: a[0], guild: a.slice(1).join(" ") }));
          await env.STATS.put("cfg:guilds", JSON.stringify(gs));
          return json({ ok: true, n: gs.length });
        }
        if (url.pathname === "/keygen") return json({ ok: true, pk: await keygen(env) });
        if (url.pathname === "/unbind") {
          const ck = "c:" + String(p.name || "");
          const c = await env.STATS.get(ck, "json");
          if (!c) return json({ ok: false, msg: "找不到這個角色" }, 404);
          c.serials = c.serials.filter((s) => s !== p.serial);
          if (c.serials.length) await env.STATS.put(ck, JSON.stringify(c)); else await env.STATS.delete(ck);
          const sk = "s:" + String(p.serial || "");
          const rec = await env.STATS.get(sk, "json");
          if (rec && rec.char === p.name) { rec.char = ""; rec.auto = false; await env.STATS.put(sk, JSON.stringify(rec)); }
          return json({ ok: true });
        }
        if (url.pathname === "/recheck") {
          const chars = await listAll(env, "c:");
          for (const c of chars) await refresh(env, c);
          return json({ ok: true, n: chars.length });
        }
      } catch (e) { return json({ ok: false, msg: String(e.message || e) }, 500); }
    }
    if (url.pathname === "/admin" || url.pathname === "/") return dashboard(env);
    return new Response("Not found", { status: 404 });
  },
};
