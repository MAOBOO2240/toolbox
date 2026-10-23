const log = (m) => fetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[ext] ' + m)).catch(() => {});
log('opened');
chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
  log('active:' + JSON.stringify((tabs || []).map((t) => t.url)));
  document.getElementById('o').textContent = (tabs || []).map((t) => t.url).join(' ');
});
chrome.tabs.query({ active: true, lastFocusedWindow: true }, (tabs) => log('lastFocused:' + JSON.stringify((tabs || []).map((t) => t.url))));
chrome.tabs.query({}, (tabs) => log('all:' + JSON.stringify((tabs || []).map((t) => [t.url, t.active, t.windowId]))));
