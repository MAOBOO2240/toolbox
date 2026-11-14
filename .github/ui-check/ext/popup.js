const log = (m) => fetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[ext] ' + m)).catch(() => {});
log('opened');
chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
  log('active:' + JSON.stringify((tabs || []).map((t) => t.url)));
  document.getElementById('o').textContent = (tabs || []).map((t) => t.url).join(' ');
});
chrome.tabs.query({ active: true, lastFocusedWindow: true }, (tabs) => log('lastFocused:' + JSON.stringify((tabs || []).map((t) => t.url))));
chrome.tabs.query({}, (tabs) => log('all:' + JSON.stringify((tabs || []).map((t) => [t.url, t.active, t.windowId]))));
// 連本機的程式（native messaging）：一個登記在 Chrome 的位置、一個登記在 Edge 的位置
for (const host of ['com.maoboo.uitest.chrome', 'com.maoboo.uitest.edge']) {
  try {
    chrome.runtime.sendNativeMessage(host, { ping: host }, (r) => log('native ' + host + ' → ' + JSON.stringify(r) + ' err=' + (chrome.runtime.lastError ? chrome.runtime.lastError.message : '')));
  } catch (e) { log('native ' + host + ' threw ' + e); }
}
