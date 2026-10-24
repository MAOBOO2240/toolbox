// 工具箱使用狀況收集端 + 公會自動授權（Cloudflare Worker）
// 綁定：KV 命名空間 STATS；Secret：ADMIN_KEY（看儀表板用的密碼）、NEXON_API_KEY（查公會用）
//
// POST /ping   程式回報：{ serial, install, version, run, os, licensed, name }（每 10 分鐘；回 { revoked } 讓程式即時鎖回去）
// POST /auto   綁定角色：{ serial, name, licensed } → { ok, code, signed, name, world, guild } 或 { ok:false, msg }
//              還沒授權的要公會在名單裡才綁、並簽授權碼；已授權的只綁定（左下角的角色）
// 以下都要 ?key=ADMIN_KEY
// GET  /admin                        儀表板
// POST /note    { serial, note }     幫某個序號加備註
// POST /cfg     { guilds }           允許的公會（每行一個公會名稱）
// POST /keygen                       產生自動授權用的金鑰（只會產生一次）
// POST /unbind  { name, serial }     解除某個角色跟某台電腦的綁定
// POST /revoke  { serial, on }       移除／恢復某台電腦的授權（程式下次回報時會自己刪掉授權碼）
// POST /check   { serial }           程式輸入授權碼前問有沒有被移除（不用 key）
// POST /recheck                      重新查所有綁定角色的公會
//
// KV 內容：s:<序號> 每台電腦；c:<角色> 綁定角色；r:<時間> 被拒絕的紀錄；cfg:guilds / cfg:sk / cfg:pk

const NEXON = "https://open.api.nexon.com/maplestorytw/v1";
const MAX_PC = 3; // 一個角色最多綁幾台電腦
const DEFAULT_GUILDS = [{ guild: "楓林小築" }];

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
// 只比對公會名稱，不管伺服器
const allowed = (gs, c) => !!c && !!c.guild && gs.some((g) => g.guild === c.guild);

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
  await env.STATS.put(k, JSON.stringify({ ...o, k, ts: new Date().toISOString() }), { expirationTtl: 90 * 86400 });
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

/* ---------- 綁定角色（公會成員順便自動授權） ---------- */
/** 把某台電腦從某個角色的綁定裡拿掉 */
async function unbindSerial(env, name, serial) {
  if (!name) return;
  const ck = "c:" + name;
  const c = await env.STATS.get(ck, "json");
  if (!c) return;
  c.serials = (c.serials || []).filter((s) => s !== serial);
  if (c.serials.length) await env.STATS.put(ck, JSON.stringify(c)); else await env.STATS.delete(ck);
}

/** 這台電腦目前綁定的角色（要真的在角色的名單裡才算） */
async function boundOf(env, rec, serial) {
  if (!rec || !rec.char) return "";
  const c = await env.STATS.get("c:" + rec.char, "json");
  return c && (c.serials || []).includes(serial) ? rec.char : "";
}

/**
 * 綁定角色：{ serial, name, licensed }
 * - 已經有授權的（作者、輸入過授權碼）：只綁定，不看公會、不簽碼
 * - 還沒授權的：公會在名單裡才綁，並簽一組授權碼回去
 * 一台電腦只綁一個角色（換角色就把舊的解掉）；一個角色最多綁 MAX_PC 台電腦
 */
async function auto(env, p) {
  const serial = String(p.serial || "").trim().toUpperCase();
  const name = String(p.name || "").trim();
  const licensed = !!p.licensed;
  if (!SERIAL_RE.test(serial)) throw new Error("序號格式不對");
  if (!name || name.length > 20) throw new Error("請輸入角色名稱");
  const sk = "s:" + serial;
  const rec = (await env.STATS.get(sk, "json")) || { serial, first: new Date().toISOString(), last: new Date().toISOString(), pings: 0, note: "" };
  if (rec.revoked) throw new Error("這台電腦的授權已被作者移除，請作者在儀表板按「恢復授權」");
  if (!licensed && !(await env.STATS.get("cfg:pk"))) throw new Error("作者還沒在儀表板產生自動授權金鑰");
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
  const inGuild = allowed(gs, info);
  if (!licensed && !inGuild) {
    await reject(env, { serial, name: info.name, world: info.world, guild: info.guild, reason: "公會不符" });
    throw new Error(`角色「${info.name}」（${info.world}）的公會是「${info.guild || "無"}」，不在允許名單裡`);
  }
  const ck = "c:" + info.name;
  const c = (await env.STATS.get(ck, "json")) || { name: info.name, serials: [], first: new Date().toISOString() };
  if (!c.serials.includes(serial)) {
    if (c.serials.length >= MAX_PC) {
      await reject(env, { serial, name: info.name, world: info.world, guild: info.guild, reason: "這個角色綁定的電腦已達上限" });
      throw new Error(`角色「${info.name}」綁定的電腦已達上限，請找作者解除`);
    }
    // 換角色：把這台從舊角色解掉
    if (rec.char && rec.char !== info.name) await unbindSerial(env, rec.char, serial);
    c.serials.push(serial);
  }
  const signed = !licensed && inGuild;
  Object.assign(c, { world: info.world, guild: info.guild, level: info.level, class: info.class, ocid: info.ocid, checked: new Date().toISOString(), err: "", viaGuild: !!c.viaGuild || signed });
  await env.STATS.put(ck, JSON.stringify(c));
  const code = signed ? await signCode(env, serial) : "";
  Object.assign(rec, { char: info.name, licensed: true, auto: signed || (!!rec.auto && rec.char === info.name) });
  await env.STATS.put(sk, JSON.stringify(rec));
  return { ok: true, code, signed, name: info.name, world: info.world, guild: info.guild };
}

/* ---------- 儀表板 ---------- */
const CSS = `
:root{--bg:#0f1117;--card:#171a23;--card2:#1e222d;--line:#262a36;--text:#e8eaf0;--muted:#8b93a7;--accent:#7dd3fc;--good:#4ade80;--bad:#f87171;--warn:#fbbf24}
*{box-sizing:border-box}
body{font:14px/1.5 -apple-system,"Segoe UI",system-ui,"Noto Sans TC",sans-serif;margin:0;padding:2rem clamp(1rem,3vw,3rem) 3rem;background:var(--bg);color:var(--text)}
h1{font-size:1.5rem;margin:0 0 1.2rem;letter-spacing:.02em}
h1 small{font-size:.8rem;color:var(--muted);font-weight:400;margin-left:.8rem}
h2{font-size:1rem;margin:2rem 0 .7rem;color:var(--muted);font-weight:600;text-transform:none;letter-spacing:.04em}
h2 .h2btn{margin-left:.8rem;font-size:.78rem;padding:.2rem .6rem;font-weight:400}
h2 .n{display:inline-block;min-width:1.5em;padding:0 .45em;margin-left:.4em;border-radius:999px;background:var(--card2);color:var(--text);font-size:.8rem;text-align:center}
.sum{display:grid;grid-template-columns:repeat(auto-fit,minmax(9.5rem,1fr));gap:.8rem;margin-bottom:1.2rem}
.sum div{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:.8rem 1rem}
.sum span{display:block;color:var(--muted);font-size:.78rem}
.sum b{display:block;font-size:1.7rem;font-weight:600;margin-top:.1rem}
.cfg{display:grid;grid-template-columns:1fr 1fr;gap:.8rem}
.cfg:has(> .card:only-child){grid-template-columns:1fr}
.pkd{margin-top:.6rem;color:var(--muted);font-size:.78rem}.pkd summary{cursor:pointer}.pkd .pk{margin-top:.4rem;max-width:40rem}
@media(max-width:900px){.cfg{grid-template-columns:1fr}}
.card{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:1rem 1.1rem}
.card h3{margin:0 0 .5rem;font-size:.9rem}
.card small{display:block;color:var(--muted);margin-top:.5rem;font-size:.78rem}
textarea,input{background:var(--card2);color:var(--text);border:1px solid var(--line);border-radius:8px;padding:.4rem .6rem;font:inherit;outline:none}
textarea:focus,input:focus{border-color:var(--accent)}
textarea{width:100%;height:5.5rem;resize:vertical;font-family:ui-monospace,Consolas,monospace}
input{width:100%;max-width:12rem;padding:.3rem .55rem}
button{background:var(--card2);color:var(--text);border:1px solid var(--line);border-radius:8px;padding:.35rem .8rem;font:inherit;cursor:pointer;white-space:nowrap}
button:hover{border-color:var(--accent);color:var(--accent)}
button.danger:hover{border-color:var(--bad);color:var(--bad)}
button.primary{background:var(--accent);color:#0b1220;border-color:var(--accent);font-weight:600}
button.primary:hover{filter:brightness(1.08);color:#0b1220}
button:disabled{opacity:.5;cursor:default}
.row{display:flex;gap:.5rem;flex-wrap:wrap;margin-top:.6rem}
.pk{font-family:ui-monospace,Consolas,monospace;font-size:.85rem;word-break:break-all;color:var(--accent);background:var(--card2);padding:.5rem .7rem;border-radius:8px;user-select:all}
.tbl{background:var(--card);border:1px solid var(--line);border-radius:12px;overflow:auto}
table{border-collapse:collapse;width:100%}
th,td{padding:.6rem .8rem;text-align:left;white-space:nowrap;border-bottom:1px solid var(--line);vertical-align:middle}
th{color:var(--muted);font-weight:500;font-size:.78rem;background:var(--card2);position:sticky;top:0}
tr:last-child td{border-bottom:0}
tbody tr:hover td{background:rgba(255,255,255,.025)}
code{font-family:ui-monospace,Consolas,monospace;color:var(--accent);font-size:.85rem;letter-spacing:.03em}
.badge{display:inline-block;padding:.1rem .55rem;border-radius:999px;font-size:.76rem;font-weight:600;border:1px solid transparent}
.b-auto{background:rgba(74,222,128,.12);color:var(--good);border-color:rgba(74,222,128,.3)}
.b-manual{background:rgba(125,211,252,.12);color:var(--accent);border-color:rgba(125,211,252,.3)}
.b-off{background:rgba(248,113,113,.12);color:var(--bad);border-color:rgba(248,113,113,.3)}
.b-none{color:var(--muted);border-color:var(--line)}
.b-good{background:rgba(74,222,128,.12);color:var(--good)}
.b-bad{background:rgba(248,113,113,.12);color:var(--bad)}
.sub{color:var(--muted);font-size:.76rem;margin-left:.35rem}
.muted{color:var(--muted)}
.actions{display:flex;gap:.35rem;justify-content:flex-end}
.empty{color:var(--muted);padding:1rem 1.1rem}
.foot{color:var(--muted);font-size:.78rem;margin-top:1.5rem;line-height:1.7}
`;

async function dashboard(env) {
  const [list, chars, rejects, gs] = await Promise.all([listAll(env, "s:"), listAll(env, "c:"), listAll(env, "r:"), guilds(env)]);
  const pk = (await env.STATS.get("cfg:pk")) || "";
  const cmap = Object.fromEntries(chars.map((c) => [c.name, c]));
  list.sort((a, b) => (b.last || "").localeCompare(a.last || ""));
  const day = 86400000, now = Date.now();
  const active = (ms) => list.filter((x) => now - Date.parse(x.last) < ms).length;
  const bad = chars.filter((c) => c.viaGuild && !allowed(gs, c));
  const licBadge = (x) => x.revoked ? `<span class="badge b-off">已移除</span>` : x.licensed ? (x.auto ? `<span class="badge b-auto">自動</span>` : `<span class="badge b-manual">手動</span>`) : `<span class="badge b-none">未授權</span>`;
  const rows = list.map((x) => {
    const c = x.char ? cmap[x.char] : null;
    const ok = c ? (c.viaGuild ? allowed(gs, c) : null) : null; // 不是靠公會授權的，公會只是顯示
    return `<tr>
      <td><code>${esc(x.serial)}</code></td>
      <td><input value="${esc(x.note)}" data-serial="${esc(x.serial)}" placeholder="這是誰" /></td>
      <td>${licBadge(x)}</td>
      <td>${x.char ? `<b>${esc(x.char)}</b>${c ? `<span class="sub">${esc(c.world)} · Lv.${esc(c.level)} ${esc(c.class)}</span>` : ""}` : `<span class="muted">—</span>`}</td>
      <td>${c ? `<span class="badge ${ok === null ? "b-none" : ok ? "b-good" : "b-bad"}">${esc(c.guild || "無")}</span>${c.err ? ` <span class="sub" title="${esc(c.err)}">⚠ 查詢失敗</span>` : ""}` : `<span class="muted">—</span>`}</td>
      <td title="${esc(x.first)}">${tw(x.first)}</td>
      <td title="${esc(x.last)}">${tw(x.last)}</td>
      <td title="${esc(x.os)}"><span class="muted">${esc(String(x.version || "").slice(0, 7))}${x.run ? " #" + esc(x.run) : ""}</span></td>
      <td><div class="actions">${x.char ? `<button data-unbind="${esc(x.char)}" data-serial="${esc(x.serial)}">解除綁定</button>` : ""}${x.revoked ? `<button data-revoke="0" data-serial="${esc(x.serial)}">恢復授權</button>` : x.licensed ? `<button class="danger" data-revoke="1" data-serial="${esc(x.serial)}">移除授權</button>` : ""}</div></td></tr>`;
  }).join("");
  const badRows = bad.map((c) => `<tr><td><b>${esc(c.name)}</b></td><td>${esc(c.world)}</td><td><span class="badge b-bad">${esc(c.guild || "無")}</span></td><td>${c.serials.map((s) => `<code>${esc(s)}</code>`).join("<br>")}</td><td title="${esc(c.checked)}">${tw(c.checked)}</td><td><div class="actions">${c.serials.map((s) => `<button data-unbind="${esc(c.name)}" data-serial="${esc(s)}">解除 …${esc(s.slice(-4))}</button>`).join("")}</div></td></tr>`).join("");
  const rejRows = rejects.slice(0, 200).map((r) => `<tr><td title="${esc(r.ts)}">${tw(r.ts)}</td><td><code>${esc(r.serial)}</code></td><td><b>${esc(r.name)}</b></td><td>${esc(r.world) || `<span class="muted">—</span>`}</td><td>${r.world ? `<span class="badge ${r.reason === "公會不符" ? "b-bad" : "b-good"}">${esc(r.guild || "無")}</span>` : `<span class="muted">—</span>`}</td><td>${esc(r.reason)}</td><td><div class="actions">${r.k ? `<button class="danger" data-rejdel="${esc(r.k)}">刪除</button>` : ""}</div></td></tr>`).join("");
  const gtext = gs.map((g) => g.guild).join("\n");
  const table = (head, body, emptyText) => `<div class="tbl">${body ? `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>` : `<div class="empty">${emptyText}</div>`}</div>`;
  return html(`<!doctype html><html lang="zh-Hant"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>工具箱使用狀況</title><style>${CSS}</style>
<h1>工具箱使用狀況<small>台灣時間 ${tw(new Date().toISOString())}</small></h1>
<div class="sum">
  <div><span>總安裝數</span><b>${list.length}</b></div>
  <div><span>今天有開</span><b>${active(day)}</b></div>
  <div><span>7 天內</span><b>${active(7 * day)}</b></div>
  <div><span>30 天內</span><b>${active(30 * day)}</b></div>
  <div><span>已授權</span><b>${list.filter((x) => x.licensed && !x.revoked).length}</b></div>
  <div><span>綁定角色</span><b>${chars.length}</b></div>
  <div><span>公會不符</span><b style="${bad.length ? "color:var(--bad)" : ""}">${bad.length}</b></div>
</div>
<div class="cfg">
  <div class="card"><h3>允許自動授權的公會</h3><textarea id="guilds" spellcheck="false">${esc(gtext)}</textarea>
    <div class="row"><button class="primary" id="saveG">儲存名單</button><button id="recheck">重新查所有人的公會</button></div>
    <small>每行一個公會名稱（不用寫伺服器）。改完名單後已綁定的人不會失效，只會列在「公會不符」。</small></div>
  ${pk ? "" : `<div class="card"><h3>自動授權金鑰（公鑰）</h3><div class="row"><button class="primary" id="keygen">產生金鑰</button></div><small>產生後把公鑰交給開發者放進程式，自動授權才會生效。</small></div>`}
</div>
${pk ? `<details class="pkd"><summary>自動授權金鑰已設定</summary><div class="pk">${esc(pk)}</div></details>` : ""}
<h2>所有電腦<span class="n">${list.length}</span></h2>
${table(`<th>序號</th><th>備註</th><th>授權</th><th>綁定角色</th><th>公會</th><th>第一次使用</th><th>最後一次使用</th><th>版本</th><th></th>`, rows, "還沒有任何電腦回報")}
<h2>公會不符<span class="n">${bad.length}</span></h2>
${table(`<th>角色</th><th>伺服器</th><th>目前公會</th><th>綁定的電腦</th><th>上次查詢</th><th></th>`, badRows, "已綁定的角色都還在名單裡的公會")}
<h2>被拒絕的自動授權<span class="n">${rejects.length}</span>${rejects.length ? `<button class="danger h2btn" id="rejAll">全部清除</button>` : ""}</h2>
${table(`<th>時間</th><th>序號</th><th>輸入的角色</th><th>伺服器</th><th>公會</th><th>原因</th><th></th>`, rejRows, "最近 90 天沒有被拒絕的嘗試")}
<p class="foot">備註改完會自動儲存。程式每 10 分鐘回報一次（最後一次使用最多晚 1 小時）：序號、版本、有沒有授權、綁定的角色；綁定角色的公會每天重查一次。<br>
「移除授權」：對方的程式最晚 10 分鐘內（開工具時會立刻再確認一次）會自動刪掉授權碼並鎖回去，也不能再用自動授權或舊授權碼；按「恢復授權」後對方再驗證一次（或重貼授權碼）就能用。「解除綁定」後對方程式會把角色清掉（公會自動授權的也會一起鎖回去），重新輸入角色名稱就能再綁。</p>
<script>
const K="?key="+encodeURIComponent(new URLSearchParams(location.search).get("key"));
const post=async(p,b)=>{const r=await fetch(p+K,{method:"POST",body:JSON.stringify(b||{})});const j=await r.json().catch(()=>({}));if(!j.ok)alert(j.msg||"失敗");return j;};
document.querySelectorAll("input[data-serial]").forEach((i)=>i.addEventListener("change",async()=>{await post("/note",{serial:i.dataset.serial,note:i.value});i.style.borderColor="var(--good)";}));
document.querySelectorAll("button[data-unbind]").forEach((b)=>b.addEventListener("click",async()=>{if(!confirm("解除「"+b.dataset.unbind+"」跟 "+b.dataset.serial+" 的綁定？"))return;await post("/unbind",{name:b.dataset.unbind,serial:b.dataset.serial});location.reload();}));
document.querySelectorAll("button[data-revoke]").forEach((b)=>b.addEventListener("click",async()=>{const on=b.dataset.revoke==="1";if(!confirm((on?"移除":"恢復")+" "+b.dataset.serial+" 的授權？"))return;await post("/revoke",{serial:b.dataset.serial,on});location.reload();}));
document.querySelectorAll("button[data-rejdel]").forEach((b)=>b.addEventListener("click",async()=>{await post("/rejdel",{k:b.dataset.rejdel});b.closest("tr").remove();}));
const ra=document.getElementById("rejAll");if(ra)ra.onclick=async()=>{if(!confirm("清除所有被拒絕的紀錄？"))return;await post("/rejdel",{all:true});location.reload();};
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
      let ch = String(p.name || old.char || "").slice(0, 20);
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
        revoked: !!old.revoked,
        char: ch,
        first: old.first || now,
        last: now,
        pings: (old.pings || 0) + 1,
        note: old.note || "",
        ip_country: req.headers.get("cf-ipcountry") || "",
      };
      // 程式每 10 分鐘回報一次，但 KV 免費額度一天只能寫 1000 次：內容沒變的話每小時才寫一次
      const changed = ["install", "version", "run", "os", "licensed", "char"].some((k) => rec[k] !== old[k]);
      if (changed || !old.last || Date.now() - Date.parse(old.last) > 3600000) await env.STATS.put(key, JSON.stringify(rec));
      return json({ ok: true, revoked: rec.revoked, bound: ch });
    }
    // 程式輸入授權碼前先問：這台有沒有被移除授權
    if (req.method === "POST" && url.pathname === "/check") {
      const p = await req.json().catch(() => ({}));
      const serial = String(p.serial || "").toUpperCase();
      const rec = await env.STATS.get("s:" + serial, "json");
      return json({ ok: true, revoked: !!(rec && rec.revoked), bound: await boundOf(env, rec, serial) });
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
          const gs = String(p.guilds || "").split(/\r?\n/).map((l) => l.trim()).filter(Boolean).map((guild) => ({ guild }));
          await env.STATS.put("cfg:guilds", JSON.stringify(gs));
          return json({ ok: true, n: gs.length });
        }
        if (url.pathname === "/keygen") return json({ ok: true, pk: await keygen(env) });
        if (url.pathname === "/revoke") {
          const sk = "s:" + String(p.serial || "");
          const rec = await env.STATS.get(sk, "json");
          if (!rec) return json({ ok: false, msg: "找不到這個序號" }, 404);
          rec.revoked = !!p.on;
          if (rec.revoked) rec.licensed = false;
          await env.STATS.put(sk, JSON.stringify(rec));
          return json({ ok: true });
        }
        if (url.pathname === "/unbind") {
          await unbindSerial(env, String(p.name || ""), String(p.serial || ""));
          const sk = "s:" + String(p.serial || "");
          const rec = await env.STATS.get(sk, "json");
          if (rec && rec.char === p.name) { rec.char = ""; rec.auto = false; await env.STATS.put(sk, JSON.stringify(rec)); }
          return json({ ok: true });
        }
        if (url.pathname === "/rejdel") {
          const keys = p.all ? (await env.STATS.list({ prefix: "r:" })).keys.map((k) => k.name) : [String(p.k || "")].filter((k) => k.startsWith("r:"));
          for (const k of keys) await env.STATS.delete(k);
          return json({ ok: true, n: keys.length });
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
