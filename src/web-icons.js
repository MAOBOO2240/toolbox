// 瀏覽器視窗共用的圖示與小工具（工具列 web.html、選單 web-menu.html、內部頁面 web-ui.html）。
// 圖示都是自己畫的線條圖示（24×24），顏色跟著文字。
"use strict";
(() => {
  const p = (d, extra) => `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true">${extra || ""}<path d="${d}"/></svg>`;
  const dot = (cx, cy, r) => `<circle cx="${cx}" cy="${cy}" r="${r}" fill="currentColor" stroke="none"/>`;
  const ring = (cx, cy, r) => `<circle cx="${cx}" cy="${cy}" r="${r}"/>`;
  const ICONS = {
    back: p("M19.5 12H5M11 5.5 4.5 12l6.5 6.5"),
    forward: p("M4.5 12H19M13 5.5l6.5 6.5-6.5 6.5"),
    reload: p("M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v5h-5"),
    stop: p("M6.5 6.5l11 11M17.5 6.5l-11 11"),
    home: p("M4.5 10.6 12 4.2l7.5 6.4v8.9h-5V14h-5v5.5h-5z"),
    tune: p("M12 8h8M4 8h2.2M4 16h8M17.8 16H20", ring(8.8, 8, 2.4) + ring(15.2, 16, 2.4)),
    search: p("M15.5 15.5 20 20", ring(10.5, 10.5, 6)),
    zoom: p("M15.5 15.5 20 20M8 10.5h5M10.5 8v5", ring(10.5, 10.5, 6)),
    star: p("M12 3.8l2.5 5.2 5.7.8-4.1 4 1 5.7-5.1-2.7-5.1 2.7 1-5.7-4.1-4 5.7-.8z"),
    "star-on": `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12 3.8l2.5 5.2 5.7.8-4.1 4 1 5.7-5.1-2.7-5.1 2.7 1-5.7-4.1-4 5.7-.8z"/></svg>`,
    puzzle: p("M9.2 4.6a2 2 0 0 1 4 0V6h3.3A1.5 1.5 0 0 1 18 7.5V11h1.4a2 2 0 0 1 0 4H18v3.5a1.5 1.5 0 0 1-1.5 1.5H13v-1.4a2 2 0 0 0-4 0V20H5.5A1.5 1.5 0 0 1 4 18.5V15h1.4a2 2 0 0 0 0-4H4V7.5A1.5 1.5 0 0 1 5.5 6h3.7z"),
    download: p("M12 4.5V15M7.5 10.8 12 15.3l4.5-4.5M5 19.5h14"),
    more: `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true">${dot(12, 5.5, 1.7)}${dot(12, 12, 1.7)}${dot(12, 18.5, 1.7)}</svg>`,
    user: p("M5 19.5c.8-3.2 3.6-5 7-5s6.2 1.8 7 5", ring(12, 8.5, 3.5)),
    incognito: p("M3 11.5h18M7.2 11.5 8.6 6h6.8l1.4 5.5M10.6 16.5h2.8", ring(8, 16.5, 2.6) + ring(16, 16.5, 2.6)),
    "chev-down": p("M7 10l5 5 5-5"),
    "chev-right": p("M10 7l5 5-5 5"),
    plus: p("M12 5.5v13M5.5 12h13"),
    close: p("M7 7l10 10M17 7 7 17"),
    folder: p("M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"),
    globe: p("M3.5 12h17M12 3.5c2.6 2.7 2.6 14.3 0 17M12 3.5c-2.6 2.7-2.6 14.3 0 17", ring(12, 12, 8.5)),
    audio: p("M4 10v4h3.5l4.5 4V6L7.5 10zM15.5 9a4.2 4.2 0 0 1 0 6M17.8 6.5a7.6 7.6 0 0 1 0 11"),
    muted: p("M4 10v4h3.5l4.5 4V6L7.5 10zM16 9.5l5 5M21 9.5l-5 5"),
    tab: p("M12 9.5v5M9.5 12h5", `<rect x="3.5" y="5.5" width="17" height="13" rx="2.2"/>`),
    window: p("M3.5 9.5h17", `<rect x="3.5" y="5" width="17" height="14" rx="2.2"/>`),
    history: p("M12 7.5V12l3 2M4.2 9.5A8 8 0 1 1 4 12M4 5.5v4h4"),
    trash: p("M4.5 7h15M10 11v6M14 11v6M6.5 7l.9 11.2a2 2 0 0 0 2 1.8h5.2a2 2 0 0 0 2-1.8L17.5 7M9.5 7V4.5h5V7"),
    print: p("M7 8.5v-4h10v4M7 16.5H5a1.5 1.5 0 0 1-1.5-1.5v-5A1.5 1.5 0 0 1 5 8.5h14a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5h-2M7 13.5h10v6H7z"),
    find: p("M15.5 15.5 20 20", ring(10.5, 10.5, 6)),
    share: p("M12 3.5v11M8 7.5l4-4 4 4M6 11.5H5a1.5 1.5 0 0 0-1.5 1.5v6A1.5 1.5 0 0 0 5 20.5h14a1.5 1.5 0 0 0 1.5-1.5v-6a1.5 1.5 0 0 0-1.5-1.5h-1"),
    save: p("M5 4.5h11l3.5 3.5v10.5a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 18.5V6A1.5 1.5 0 0 1 5 4.5zM7.5 4.5v5h7v-5M7.5 20v-6h9v6"),
    tools: p("M14.5 5.5a4 4 0 0 0-5.3 5.2L4 16v4h4l5.3-5.2a4 4 0 0 0 5.2-5.3l-2.8 2.8-2.4-.6-.6-2.4z"),
    gear: p("M12 3.5l1.6 2.3 2.7-.5.9 2.6 2.6.9-.5 2.7 2.2 1.5-2.2 1.5.5 2.7-2.6.9-.9 2.6-2.7-.5L12 22.5l-1.6-2.3-2.7.5-.9-2.6-2.6-.9.5-2.7L2.5 13l2.2-1.5-.5-2.7 2.6-.9.9-2.6 2.7.5z", ring(12, 13, 3)),
    exit: p("M14.5 4.5H6A1.5 1.5 0 0 0 4.5 6v12A1.5 1.5 0 0 0 6 19.5h8.5M10 12h10M16.5 8.5 20 12l-3.5 3.5"),
    edit: p("M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4"),
    link: p("M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"),
    open: p("M13.5 4.5h6v6M19.5 4.5 11 13M17.5 13.5v5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1h5"),
    code: p("M8.5 7 3.5 12l5 5M15.5 7l5 5-5 5"),
    task: p("M4 6.5h16M4 12h16M4 17.5h10"),
    lock: p("M8 10.5V8a4 4 0 0 1 8 0v2.5", `<rect x="5.5" y="10.5" width="13" height="9.5" rx="2"/>`),
    warn: p("M12 4 2.8 19.5h18.4zM12 10v4.5M12 17.2v.3"),
    sync: p("M19 8.5A7.5 7.5 0 0 0 5.6 7M5 4v4h4M5 15.5A7.5 7.5 0 0 0 18.4 17M19 20v-4h-4"),
    check: p("M5 12.5l4.5 4.5L19 7.5"),
    pin: p("M9 4.5h6l-.8 6 3.3 3.5H6.5L9.8 10.5zM12 14v6"),
    copy: p("M8 8V5.5A1.5 1.5 0 0 1 9.5 4h9A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H16", `<rect x="4" y="8" width="12" height="12" rx="1.5"/>`),
    full: p("M4.5 9V4.5H9M15 4.5h4.5V9M19.5 15v4.5H15M9 19.5H4.5V15"),
    minus: p("M6 12h12"),
    file: p("M6.5 3.5h7l4 4v13h-11zM13.5 3.5v4h4"),
    eye: p("M2.5 12s3.5-6.5 9.5-6.5S21.5 12 21.5 12s-3.5 6.5-9.5 6.5S2.5 12 2.5 12z", ring(12, 12, 3)),
    script: p("M8 7 4 12l4 5M16 7l4 5-4 5M13.5 5l-3 14"),
    clock: p("M12 7v5l3.2 2", ring(12, 12, 8.5)),
    "new-folder": p("M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2zM12 10.5v5M9.5 13h5"),
  };

  const host = (u) => { try { return new URL(u).host; } catch { return ""; } };

  function letter(text) {
    const el = document.createElement("span");
    let h = 0;
    for (const c of text || "?") h = (h * 31 + c.charCodeAt(0)) % 360;
    el.className = "letter";
    el.style.background = `hsl(${h} 45% 42%)`;
    el.textContent = ((text || "?").trim().charAt(0) || "?").toUpperCase();
    return el;
  }

  // 圖示：內建圖示的名稱、圖片網址，或都沒有（用第一個字）
  function icon(src, text) {
    if (src && ICONS[src]) {
      const s = document.createElement("span");
      s.className = "gl";
      s.innerHTML = ICONS[src];
      return s;
    }
    if (!src || !/^(https?:|data:image\/)/i.test(src)) return letter(text);
    const img = document.createElement("img");
    img.className = "fav";
    img.alt = "";
    img.draggable = false;
    img.referrerPolicy = "no-referrer";
    img.onerror = () => img.replaceWith(letter(text));
    img.src = src;
    return img;
  }

  window.MB = { ICONS, host, letter, icon };
})();
