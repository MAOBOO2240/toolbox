"use strict";

/* ==========================================================
 * web：置頂的小瀏覽器（浮動視窗）。這一頁管理有哪些視窗、開關與設定；
 * 視窗本身由 Rust 建立，上面的工具列是 web.html。
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const wapi = TAURI ? {
    list: () => invoke("web_list"),
    add: () => invoke("web_add"),
    open: (id) => invoke("web_open", { id }),
    close: (id) => invoke("web_close", { id }),
    del: (id) => invoke("web_delete", { id }),
    set: (id, o) => invoke("web_set", { id, opacity: o.opacity ?? null, locked: o.locked ?? null, gameOnly: o.game_only ?? null, chrome: o.chrome ?? null }),
    chrome: () => invoke("web_chrome_status"),
    shot: (id) => invoke("web_shot", { id }),
    importChrome: (dir) => invoke("web_chrome_import", { dir: dir ?? null }),
    sync: (on) => invoke("web_chrome_sync", { on }),
    exts: () => invoke("web_ext_list"),
    scripts: () => invoke("web_scripts_list"),
    scriptsImport: () => invoke("web_scripts_import"),
    scriptsToggle: (file, on) => invoke("web_scripts_toggle", { file, on }),
    scriptsDelete: (file) => invoke("web_scripts_delete", { file }),
    scriptsDir: () => invoke("web_scripts_open_dir"),
    scriptsReload: () => invoke("web_scripts_reload"),
    extSync: () => invoke("web_ext_sync"),
    onChange: (cb) => listen("web-changed", cb),
  } : mockApi();

  let file = { next_id: 0, wins: [], bookmarks: { bar: [], other: [] } };
  const countLinks = (list) => (list || []).reduce((n, b) => n + (b.url ? 1 : countLinks(b.children)), 0);

  const host = (u) => { try { return new URL(u).host; } catch { return u; } };
  function letterIcon(text) {
    const el = document.createElement("i");
    let h = 0;
    for (const c of text || "?") h = (h * 31 + c.charCodeAt(0)) % 360;
    el.className = "web-letter";
    el.style.background = `hsl(${h} 55% 48%)`;
    el.textContent = (text || "?").charAt(0).toUpperCase();
    return el;
  }
  const sw = (cls, on) => `<label class="switch sm"><input type="checkbox" class="${cls}"${on ? " checked" : ""} /><span class="slider"></span></label>`;

  function render() {
    const list = $("#webList");
    list.innerHTML = "";
    if (!file.wins.length) {
      list.innerHTML = '<div class="g-empty">還沒有任何視窗，按右上角「新增視窗」開一個（預設首頁是 Google）</div>';
      return;
    }
    file.wins.forEach((w, i) => {
      const el = document.createElement("div");
      el.className = `web-card${w.open ? " open" : ""}${w.open && w.min ? " mini" : ""}`;
      el.dataset.id = w.id;
      const tabs = w.tabs || [];
      const state = !w.open ? "未開啟" : w.min ? "已縮小到工作列" : "開啟中";
      el.classList.toggle("chrome", !!w.chrome);
      el.innerHTML = `
        <div class="web-win">
          <div class="web-titlebar">
            <span class="web-dots"><i></i><i></i><i></i></span>
            <span class="web-wtitle"></span>
            ${w.chrome ? '<span class="web-kind" title="用你電腦上的 Chrome 開：帳號、書籤、擴充功能都是 Chrome 的">Chrome</span>' : ""}
            <span class="web-state">${state}</span>
          </div>
          <div class="web-tabsrow"${w.chrome ? " hidden" : ""}></div>
          <div class="web-screen">
            <img class="web-shot" alt="" draggable="false" hidden />
            <span class="web-no">${i + 1}</span>
            <span class="web-host muted"></span>
          </div>
        </div>
        <div class="web-actions">
          ${w.open && w.min ? '<button class="cbtn small c-blue solid web-show">顯示視窗</button>' : ""}
          <button class="cbtn small ${w.open ? "c-silver" : "c-green solid"} web-toggle">${w.open ? "關閉視窗" : "打開視窗"}</button>
          <button class="cbtn small c-red web-del">刪除</button>
        </div>
        <div class="web-opts">
          <label class="web-opt" title="開啟：用你電腦上的 Chrome 開一般的 Chrome 視窗（帳號、書籤、擴充功能都是 Chrome 的）。關閉：用工具箱內建的瀏覽器"><span>Chrome 視窗</span>${sw("web-chrome", w.chrome)}</label>
          <div class="web-opt web-builtin"><span>不透明度</span>
            <span class="web-range"><input type="range" class="range web-alpha" min="30" max="100" step="5" value="${w.opacity}" /><b>${w.opacity}%</b></span></div>
          <label class="web-opt web-builtin" title="鎖定後滑鼠會直接穿過視窗點到遊戲，視窗本身就不能點了；要操作時回到這裡解除"><span>鎖定（滑鼠穿透）</span>${sw("web-lock", w.locked)}</label>
        </div>`;
      el.querySelector(".web-wtitle").textContent = w.title || "新視窗";
      // 視窗的畫面：開著時最近抓到的、關了的最後一張
      wapi.shot(w.id).then((src) => {
        if (!src || !el.isConnected) return;
        const im = el.querySelector(".web-shot");
        im.src = src; im.hidden = false;
        el.classList.add("has-shot");
      }).catch(() => {});
      el.querySelector(".web-host").textContent = w.chrome ? host(w.url) : [host(w.url), `${Math.max(1, tabs.length)} 個分頁`].filter(Boolean).join("・");
      el.querySelector(".web-win").title = w.url;
      // 分頁列：每個分頁的網站圖示（抓不到就用第一個字）
      const row = el.querySelector(".web-tabsrow");
      (tabs.length ? tabs : [{ id: "", url: w.url, title: w.title, icon: "" }]).slice(0, 8).forEach((t) => {
        const tab = document.createElement("span");
        tab.className = "web-mtab" + (t.id === w.active ? " on" : "");
        tab.title = t.title || t.url;
        const text = (t.title || host(t.url) || "?").trim();
        if (t.icon) {
          const img = document.createElement("img");
          img.src = t.icon; img.alt = ""; img.draggable = false;
          img.onerror = () => { img.replaceWith(letterIcon(text)); };
          tab.append(img);
        } else tab.append(letterIcon(text));
        const tt = document.createElement("span");
        tt.className = "web-mtt";
        tt.textContent = text;
        tab.append(tt);
        row.append(tab);
      });
      if (tabs.length > 8) { const m = document.createElement("span"); m.className = "web-mtab more"; m.textContent = `+${tabs.length - 8}`; row.append(m); }

      el.querySelector(".web-show")?.addEventListener("click", () => run(async () => {
        await wapi.open(w.id);
        await reload();
      }));
      el.querySelector(".web-toggle").addEventListener("click", () => run(async () => {
        if (w.open) await wapi.close(w.id); else await wapi.open(w.id);
        await reload();
      }));
      el.querySelector(".web-del").addEventListener("click", () => run(async () => {
        if (!(await confirmBox(`確定要刪除「${w.title || "新視窗"}」嗎？`))) return;
        await wapi.del(w.id);
        await reload();
      }));
      const alpha = el.querySelector(".web-alpha");
      alpha.addEventListener("input", () => {
        w.opacity = +alpha.value;
        alpha.nextElementSibling.textContent = `${w.opacity}%`;
        run(() => wapi.set(w.id, { opacity: w.opacity }));
      });
      // 游標放在滑桿上可以用滾輪調整
      alpha.addEventListener("wheel", (e) => {
        e.preventDefault();
        alpha.value = Math.max(30, Math.min(100, +alpha.value + (e.deltaY < 0 ? 5 : -5)));
        alpha.dispatchEvent(new Event("input"));
      }, { passive: false });
      el.querySelector(".web-chrome").addEventListener("change", (e) => run(async () => {
        w.chrome = e.target.checked;
        await wapi.set(w.id, { chrome: w.chrome });
        await reload();
        toast(w.chrome ? "這個視窗改用 Chrome 開" : "這個視窗改用內建瀏覽器");
      }));
      el.querySelector(".web-lock").addEventListener("change", (e) => {
        w.locked = e.target.checked;
        run(() => wapi.set(w.id, { locked: w.locked }));
        if (w.locked) toast("已鎖定：滑鼠會穿過視窗，要操作時回到這裡解除");
      });
      list.append(el);
    });
  }

  function renderBm() {
    const b = file.bookmarks || { bar: [], other: [] };
    const n = countLinks(b.bar) + countLinks(b.other);
    const t = n ? `${n} 個書籤（所有瀏覽器視窗共用）` : "還沒有書籤";
    $("#webBmCount").textContent = t;
    $("#webBmCount2").textContent = t;
  }

  async function reload() {
    file = await wapi.list();
    render();
    renderBm();
    await renderChrome();
    renderExt(await wapi.exts());
    renderScripts(await wapi.scripts());
  }

  /* ---------- 使用者腳本 ---------- */
  function renderScripts(x) {
    const list = (x && x.list) || [];
    const on = list.filter((s) => !s.off).length;
    const t = list.length ? `${list.length} 個（${on} 個啟用）` : "還沒有腳本";
    $("#webScriptCount").textContent = t;
    $("#webScriptSum").textContent = t;
    const box = $("#webScripts");
    box.innerHTML = "";
    for (const sc of list) {
      const el = document.createElement("div");
      el.className = "web-script" + (sc.off ? " off" : "");
      const where = [...(sc.meta.matches || []), ...(sc.meta.includes || [])].join("、");
      el.innerHTML = `<label class="switch sm"><input type="checkbox" ${sc.off ? "" : "checked"} /><span class="slider"></span></label>
        <div class="web-script-name"><strong></strong><span class="muted"></span></div>
        <button class="web-script-del" title="刪除這個腳本">×</button>`;
      el.querySelector("strong").textContent = sc.meta.name + (sc.meta.version ? ` v${sc.meta.version}` : "");
      const sub = el.querySelector(".muted");
      if (sc.missing && sc.missing.length) { sub.textContent = `有外部檔案沒下載到，不會執行：${sc.missing.join("、")}`; sub.classList.add("bad"); }
      else sub.textContent = where;
      el.title = sc.meta.description || where;
      el.querySelector("input").addEventListener("change", (e) => run(async () => { await wapi.scriptsToggle(sc.file, e.target.checked); renderScripts(await wapi.scripts()); }));
      el.querySelector(".web-script-del").addEventListener("click", () => run(async () => {
        if (!(await confirmBox(`確定要刪除腳本「${sc.meta.name}」嗎？`))) return;
        await wapi.scriptsDelete(sc.file);
        renderScripts(await wapi.scripts());
      }));
      box.append(el);
    }
  }
  $("#webScriptImport").addEventListener("click", () => run(async () => {
    const msg = await wapi.scriptsImport();
    if (msg) toast(msg);
    renderScripts(await wapi.scripts());
  }));
  $("#webScriptDir").addEventListener("click", () => run(() => wapi.scriptsDir()));
  $("#webScriptReload").addEventListener("click", () => run(async () => renderScripts(await wapi.scriptsReload())));

  /* ---------- 擴充功能 ---------- */
  function renderExt(x) {
    $("#webExtCount").textContent = $("#webExtCount2").textContent = x.broken
      ? "這台電腦的 WebView2 沒辦法載入擴充功能（可以更新 Windows 或 Microsoft Edge 後再試）"
      : x.list.length ? `${x.list.length} 個（從 Chrome 同步）` : "沒有擴充功能（Chrome 裡沒有啟用中的擴充功能）";
    const box = $("#webExts");
    box.innerHTML = "";
    x.list.forEach((e, i) => {
      const chip = document.createElement("span");
      chip.className = `web-ext ${COLORS[i % COLORS.length]}${e.off ? " off" : ""}`;
      chip.textContent = e.off ? `${e.name}（已停用）` : e.name;
      chip.title = `${e.name} ${e.version}`;
      box.append(chip);
    });
  }
  // 書籤與擴充功能設定：獨立視窗
  const setupBack = $("#webSetupBack");
  const closeSetup = () => setupBack.classList.remove("show");
  $("#webSetup").addEventListener("click", () => setupBack.classList.add("show"));
  $("#webSetupClose").addEventListener("click", closeSetup);
  setupBack.addEventListener("click", (e) => { if (e.target === setupBack) closeSetup(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && setupBack.classList.contains("show")) closeSetup(); });
  $("#webExtSync").addEventListener("click", () => run(async () => {
    const x = await wapi.extSync();
    renderExt(x);
    toast(`已同步 ${x.list.length} 個擴充功能，新開的分頁會套用`);
  }));

  /* ---------- 從 Chrome 匯入書籤 ---------- */
  const COLORS = ["c-blue", "c-purple", "c-teal", "c-pink", "c-indigo", "c-orange", "c-green"];
  const who = (p) => `${p.name}${p.account ? `（${p.account}）` : ""}`;
  let chrome = { sync: true, current: null, profiles: [], error: "" };

  async function renderChrome() {
    chrome = await wapi.chrome();
    $("#webSync").checked = chrome.sync;
    const el = $("#webChromeWho");
    el.classList.toggle("bad", !!chrome.error);
    el.textContent = chrome.error || (chrome.current ? `Chrome 帳號：${who(chrome.current)}・${chrome.current.count} 個書籤` : "");
    $("#webPick").hidden = chrome.profiles.length < 2;
  }
  async function importFrom(dir) {
    const n = await wapi.importChrome(dir);
    $("#webProfiles").hidden = true;
    toast(n ? `已匯入 ${n} 個書籤` : "沒有新的書籤（都已經匯入過了）");
    await reload();
  }
  $("#webImport").addEventListener("click", () => run(() => importFrom(null)));
  $("#webSync").addEventListener("change", (e) => run(async () => {
    await wapi.sync(e.target.checked);
    toast(e.target.checked ? "已開啟自動匯入" : "已關閉自動匯入");
    await reload();
  }));
  // Chrome 有好幾個使用者：選要用哪一個帳號的書籤
  $("#webPick").addEventListener("click", () => {
    const box = $("#webProfiles");
    if (!box.hidden) { box.hidden = true; return; }
    box.innerHTML = '<span class="muted">選要用哪一個 Chrome 帳號的書籤：</span>';
    chrome.profiles.forEach((p, i) => {
      const b = document.createElement("button");
      b.className = `cbtn small ${COLORS[i % COLORS.length]}${chrome.current && chrome.current.dir === p.dir ? " solid" : ""}`;
      b.textContent = `${who(p)}・${p.count} 個書籤`;
      b.addEventListener("click", () => run(() => importFrom(p.dir)));
      box.append(b);
    });
    box.hidden = false;
  });

  $("#webAdd").addEventListener("click", () => run(async () => {
    await wapi.add();
    await reload();
  }));

  // 視窗標題、網址改變或被關掉時更新（只在這一頁顯示時才重畫）
  // 視窗開著時不跟著每個分頁變動重畫（省資源）；只有視窗打開、關閉、縮小狀態改變時才更新，
  // 關閉時卡片上留的就是最後的畫面
  const stateKey = (f) => (f.wins || []).map((w) => `${w.id}:${w.open ? 1 : 0}${w.min ? 1 : 0}`).join("|");
  wapi.onChange(() => {
    if (!$("#view-web").classList.contains("active")) return;
    run(async () => {
      const next = await wapi.list();
      if (stateKey(next) === stateKey(file)) return;
      file = next;
      render();
      renderBm();
    });
  });
  document.addEventListener("viewchange", (e) => { if (e.detail === "web") run(reload); });
  // 開著的視窗：卡片上的畫面每 3 秒換一張（只換圖，不重畫卡片）
  setInterval(() => {
    if (!$("#view-web").classList.contains("active")) return;
    for (const w of file.wins || []) {
      if (!w.open) continue;
      const el = [...$("#webList").children].find((c) => c.dataset.id === w.id);
      if (!el) continue;
      wapi.shot(w.id).then((src) => {
        if (!src || !el.isConnected) return;
        const im = el.querySelector(".web-shot");
        if (im.src !== src) im.src = src;
        im.hidden = false;
        el.classList.add("has-shot");
      }).catch(() => {});
    }
  }, 3000);
  run(reload);

  /* ---------- 瀏覽器預覽用的模擬後端 ---------- */
  function mockApi() {
    const f = {
      next_id: 1,
      wins: [{ id: "w1", url: "https://www.youtube.com/watch?v=x", title: "YouTube", opacity: 100, locked: false, game_only: false, open: true, min: true, tabs: [{ id: "t1" }, { id: "t2" }] }],
      bookmarks: { bar: [], other: [] },
    };
    const clone = (x) => JSON.parse(JSON.stringify(x));
    return {
      list: async () => clone(f),
      add: async () => { f.next_id++; const w = { id: `w${f.next_id}`, url: "https://www.google.com/", title: "", opacity: 100, locked: false, game_only: false, open: true, chrome: true }; f.wins.push(w); return clone(w); },
      open: async (id) => { Object.assign(f.wins.find((w) => w.id === id), { open: true, min: false }); },
      close: async (id) => { f.wins.find((w) => w.id === id).open = false; },
      del: async (id) => { f.wins = f.wins.filter((w) => w.id !== id); },
      set: async (id, o) => { Object.assign(f.wins.find((w) => w.id === id), o); },
      shot: async () => "",
      chrome: async () => {
        const profiles = [{ dir: "Default", name: "個人", account: "player@example.com", count: 24 }, { dir: "Profile 1", name: "工作", account: "", count: 7 }];
        return { sync: f.sync !== false, current: profiles[f.dir === "Profile 1" ? 1 : 0], profiles, error: "" };
      },
      importChrome: async (dir) => { if (dir) f.dir = dir; f.bookmarks.bar.push({ url: "https://www.youtube.com/" }, { url: "https://forum.gamer.com.tw/" }); return 2; },
      sync: async (on) => { f.sync = on; },
      exts: async () => ({ broken: false, list: [{ id: "a", name: "uBlock Origin Lite", version: "1" }, { id: "b", name: "Dark Reader", version: "4" }, { id: "c", name: "Google 翻譯", version: "2" }] }),
      extSync: async () => ({ broken: false, list: [{ id: "a", name: "uBlock Origin Lite", version: "1" }] }),
      scripts: async () => ({ list: f.scripts || [], dir: "" }),
      scriptsImport: async () => { f.scripts = [{ file: "a.user.js", off: false, missing: [], meta: { name: "YouTube Shorts 自動下一部", version: "1.2", matches: ["https://www.youtube.com/*"], includes: [] } }]; return "已匯入 1 個腳本"; },
      scriptsToggle: async (file, on) => { const x = (f.scripts || []).find((s) => s.file === file); if (x) x.off = !on; },
      scriptsDelete: async (file) => { f.scripts = (f.scripts || []).filter((s) => s.file !== file); },
      scriptsDir: async () => {},
      scriptsReload: async () => ({ list: f.scripts || [], dir: "" }),
      onChange: () => {},
    };
  }
})();
