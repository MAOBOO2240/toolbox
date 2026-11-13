"use strict";

/* ==========================================================
 * 瀏覽器：這一頁管理有哪些視窗、開關、不透明度與鎖定；
 * 視窗本身由 Rust 建立（外觀與操作比照 Chrome），上面的工具列是 web.html。
 * 書籤、擴充功能每次開啟視窗都會自動跟 Chrome 同步，管理都在瀏覽器視窗裡（⋮ 選單）。
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const wapi = TAURI ? {
    list: () => invoke("web_list"),
    add: () => invoke("web_add"),
    open: (id) => invoke("web_open", { id }),
    close: (id) => invoke("web_close", { id }),
    del: (id) => invoke("web_delete", { id }),
    set: (id, o) => invoke("web_set", { id, opacity: o.opacity ?? null, locked: o.locked ?? null, gameOnly: o.game_only ?? null }),
    shot: (id) => invoke("web_shot", { id }),
    onChange: (cb) => listen("web-changed", cb),
  } : mockApi();

  let file = { next_id: 0, wins: [] };

  const PAGES = { newtab: "新分頁", history: "歷史記錄", downloads: "下載", bookmarks: "書籤", extensions: "擴充功能", settings: "設定" };
  // 網站的網域；瀏覽器自己的頁面（chrome://…）顯示名稱
  const host = (u) => {
    const m = /^chrome:\/\/([a-z-]+)/.exec(u || "");
    if (m) return PAGES[m[1]] || "";
    try { return new URL(u).host; } catch { return u; }
  };
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
      list.innerHTML = '<div class="g-empty">還沒有任何視窗，按右上角「新增視窗」開一個</div>';
      return;
    }
    file.wins.forEach((w, i) => {
      const el = document.createElement("div");
      el.className = `web-card${w.open ? " open" : ""}${w.open && w.min ? " mini" : ""}`;
      el.dataset.id = w.id;
      const tabs = w.tabs || [];
      const state = !w.open ? "未開啟" : w.min ? "已縮小到工作列" : "開啟中";
      el.innerHTML = `
        <div class="web-win">
          <div class="web-titlebar">
            <span class="web-dots"><i></i><i></i><i></i></span>
            <span class="web-wtitle"></span>
            <span class="web-state">${state}</span>
          </div>
          <div class="web-tabsrow"></div>
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
          <div class="web-opt"><span>不透明度</span>
            <span class="web-range"><input type="range" class="range web-alpha" min="30" max="100" step="5" value="${w.opacity}" /><b>${w.opacity}%</b></span></div>
          <label class="web-opt" title="鎖定後滑鼠會直接穿過視窗點到遊戲，視窗本身就不能點了；要操作時回到這裡解除"><span>鎖定（滑鼠穿透）</span>${sw("web-lock", w.locked)}</label>
        </div>`;
      el.querySelector(".web-wtitle").textContent = w.title || "新分頁";
      // 視窗的畫面：開著時最近抓到的、關了的最後一張
      wapi.shot(w.id).then((src) => {
        if (!src || !el.isConnected) return;
        const im = el.querySelector(".web-shot");
        im.src = src; im.hidden = false;
        el.classList.add("has-shot");
      }).catch(() => {});
      el.querySelector(".web-host").textContent = [host(w.url), `${Math.max(1, tabs.length)} 個分頁`].filter(Boolean).join("・");
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
        if (!(await confirmBox(`確定要刪除「${w.title || "新分頁"}」嗎？`))) return;
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
      el.querySelector(".web-lock").addEventListener("change", (e) => {
        w.locked = e.target.checked;
        run(() => wapi.set(w.id, { locked: w.locked }));
        if (w.locked) toast("已鎖定：滑鼠會穿過視窗，要操作時回到這裡解除");
      });
      list.append(el);
    });
  }

  async function reload() {
    file = await wapi.list();
    render();
  }

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
    };
    const clone = (x) => JSON.parse(JSON.stringify(x));
    return {
      list: async () => clone(f),
      add: async () => { f.next_id++; const w = { id: `w${f.next_id}`, url: "chrome://newtab/", title: "", opacity: 100, locked: false, game_only: false, open: true }; f.wins.push(w); return clone(w); },
      open: async (id) => { Object.assign(f.wins.find((w) => w.id === id), { open: true, min: false }); },
      close: async (id) => { f.wins.find((w) => w.id === id).open = false; },
      del: async (id) => { f.wins = f.wins.filter((w) => w.id !== id); },
      set: async (id, o) => { Object.assign(f.wins.find((w) => w.id === id), o); },
      shot: async () => "",
      onChange: () => {},
    };
  }
})();
