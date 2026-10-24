# 使用狀況收集端 + 公會自動授權（Cloudflare Worker）

免費額度就夠用。設定一次就好：

1. 到 https://dash.cloudflare.com 登入（沒有帳號就免費註冊）。
2. 左邊「Workers & Pages」→「Create」→「Create Worker」，名稱取 `maboo-stats`，按 Deploy。
3. 按「Edit code」，把 `stats-worker.js` 的內容整個貼上去取代，按 Deploy。
4. 左邊「Storage & Databases」→「KV」→「Create a namespace」，名稱 `STATS`。
5. 回到 Worker →「Settings」→「Bindings」→「Add」→「KV namespace」：Variable name 填 `STATS`，選剛建的命名空間，儲存。
6. 同一頁「Variables and Secrets」→「Add」：名稱 `ADMIN_KEY`，值填一串自己想的密碼（看儀表板用），型別選 Secret，儲存。
7. 再「Add」一個：名稱 `NEXON_API_KEY`，值填 Nexon Open API 金鑰（查公會用），型別選 Secret，儲存。
8. Worker 的網址在 Overview 頁，長得像 `https://maboo-stats.xxxx.workers.dev`。把這個網址填進程式 `src-tauri/src/stats.rs` 的 `STATS_URL`。

看儀表板：`https://maboo-stats.xxxx.workers.dev/admin?key=你的密碼`

## 公會自動授權

- 儀表板上方「允許自動授權的公會」：每行「伺服器 公會名稱」，隨時可以改、可以加好幾個。
- 儀表板「自動授權金鑰」按「產生金鑰」（只能一次），把顯示的公鑰填進程式 `src-tauri/src/license.rs` 的 `PUBKEY2_HEX`，重新發布程式後自動授權才會生效。
- 使用者在程式輸入角色名稱 → Worker 查 Nexon API 的伺服器與公會 → 在名單裡就簽授權碼回去。一個角色最多綁 3 台電腦。
- 儀表板會顯示每台電腦綁定的角色和公會（每天重查一次）、公會已經不在名單的人、被拒絕的嘗試（別的公會、查不到、超過 3 台）。
- 「解除綁定」只是放出名額；授權碼是永久的，對方電腦上已經存的不會失效。
- 改 Worker 程式碼：「Edit code」貼上新內容再 Deploy，KV 裡的資料不會不見。
