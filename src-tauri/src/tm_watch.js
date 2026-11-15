(() => {
  // 篡改猴的存放區一有變動（使用者新增、編輯、開關、刪除腳本）就改這份文件的標題，
  // 程式那邊看到標題變了就再讀一次。標題裡只有「第幾次」，沒有任何內容
  if (window.__mbTmWatch) return "already";
  if (!(typeof chrome === "object" && chrome && chrome.storage && chrome.storage.onChanged)) return "no-storage";
  let n = 0;
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== "local") return;
    // 認得的存法：跟腳本無關的變動（篡改猴自己記的時間之類）不用通知
    if (window.__mbTmUsual !== false && !Object.keys(changes).some((k) => /@(source|meta|st)#/.test(k))) return;
    n += 1;
    try {
      if (!document.head) document.documentElement.insertBefore(document.createElement("head"), document.documentElement.firstChild);
      document.title = __MARK__ + n;
    } catch (e) {}
  });
  window.__mbTmWatch = true;
  return "ok";
})()
