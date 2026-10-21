//! 更新道具：記下遊戲裡所有道具的編號，遊戲改版後列出新增了哪些道具。
//!
//! - 每次程式開啟（和打開工具）時檢查 String 資料夾的檔案有沒有變，有變才重新讀
//! - 第一次只記錄目前版本；之後道具變多了，就把多出來的道具記成一筆「v282 → v283」
//! - 程式內建 v282 的道具編號：第一次使用時遊戲已經是新版本，也能直接列出 v282 之後新增的道具

use crate::avatar;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 內建的 v282 道具編號（由小到大，相鄰差值用 LEB128 存）
const BUILTIN_VERSION: u32 = 282;
static BUILTIN: &[u8] = include_bytes!("../data/items_v282.bin");

/// 最多保留幾筆改版紀錄
const KEEP: usize = 20;

#[derive(Serialize, Deserialize, Default, Clone)]
struct Snap {
    version: u32,
    /// String 資料夾檔案的大小與修改時間（沒變就不用重新讀）
    sig: String,
    /// 記錄這個版本的時間（毫秒）
    at: u64,
    ids: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NewItem {
    pub id: u32,
    /// 道具所在的資料夾，例如 Character/Cap、Item/Consume
    pub folder: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Record {
    pub from: u32,
    pub to: u32,
    /// 發現的時間（毫秒）
    pub at: u64,
    pub items: Vec<NewItem>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Store {
    snap: Option<Snap>,
    records: Vec<Record>,
}

#[derive(Serialize)]
pub struct UpdatesOut {
    /// 目前遊戲版本、道具數與記錄時間
    pub version: u32,
    pub total: usize,
    pub since: u64,
    pub records: Vec<Record>,
}

static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

pub fn init(path: PathBuf) {
    let _ = PATH.set(path);
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn load() -> Store {
    PATH.get().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(st: &Store) {
    if let (Some(p), Ok(s)) = (PATH.get(), serde_json::to_string(st)) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, s);
    }
}

/// 遊戲的 String 資料夾檔案簽章（檔名、大小、修改時間）
fn signature(game: &Path) -> String {
    let data = if game.join("Data").is_dir() { game.join("Data") } else { game.to_path_buf() };
    let mut parts: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(data.join("String")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".wz") || name.ends_with(".ini")) {
                continue;
            }
            if let Ok(m) = e.metadata() {
                let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                parts.push(format!("{name}:{}:{t}", m.len()));
            }
        }
    }
    parts.sort();
    parts.join("|")
}

/// LEB128 差值編碼還原成編號
fn decode_ids(b: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut cur, mut v, mut shift) = (0u32, 0u32, 0u32);
    for &x in b {
        v |= ((x & 0x7F) as u32) << shift;
        if x & 0x80 != 0 {
            shift += 7;
            continue;
        }
        cur = cur.wrapping_add(v);
        out.push(cur);
        v = 0;
        shift = 0;
    }
    out
}

#[cfg(test)]
fn encode_ids(ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = 0u32;
    for &id in ids {
        let mut d = id - prev;
        prev = id;
        loop {
            let b = (d & 0x7F) as u8;
            d >>= 7;
            if d == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    out
}

/// 新版本比舊版本多出來的道具（依資料夾、編號排列）
fn added(old: &[u32], items: &[(u32, &'static str, String)]) -> Vec<NewItem> {
    let old: HashSet<u32> = old.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out: Vec<NewItem> = items
        .iter()
        .filter(|(id, _, _)| !old.contains(id) && seen.insert(*id))
        .map(|(id, folder, name)| NewItem { id: *id, folder: folder.to_string(), name: name.clone() })
        .collect();
    out.sort_by_key(|x| x.id);
    out
}

/// 檢查遊戲道具有沒有變：有新增就記成一筆改版紀錄
fn check(hint: Option<&str>, force: bool) -> Result<UpdatesOut, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = avatar::game_dir(hint).ok_or("找不到新楓之谷安裝資料夾，請到「美容美髮」的「解析道具」頁選擇遊戲資料夾")?;
    let sig = signature(&dir);
    let mut st = load();
    let fresh = st.snap.as_ref().is_some_and(|s| s.sig == sig && !sig.is_empty());
    if !fresh || force {
        let (version, items) = avatar::all_items(hint)?;
        if items.is_empty() {
            return Err("遊戲資料裡讀不到道具".into());
        }
        let mut ids: Vec<u32> = items.iter().map(|x| x.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let now = now_ms();
        // 比較的對象：上次記錄的版本；第一次使用、而且遊戲已經比內建的版本新時，用內建的 v282
        let base: Option<(u32, Vec<u32>)> = match &st.snap {
            Some(s) => Some((s.version, s.ids.clone())),
            None if version > BUILTIN_VERSION => Some((BUILTIN_VERSION, decode_ids(BUILTIN))),
            None => None,
        };
        let mut since = st.snap.as_ref().map(|s| s.at).unwrap_or(now);
        if let Some((from, old)) = base {
            let new = added(&old, &items);
            if !new.is_empty() {
                // 同一次改版之後又有小更新：併進同一筆
                match st.records.first_mut() {
                    Some(r) if r.from == from && r.to == version => {
                        let have: HashSet<u32> = r.items.iter().map(|x| x.id).collect();
                        r.items.extend(new.into_iter().filter(|x| !have.contains(&x.id)));
                        r.items.sort_by_key(|x| x.id);
                        r.at = now;
                    }
                    _ => st.records.insert(0, Record { from, to: version, at: now, items: new }),
                }
                st.records.truncate(KEEP);
            }
            if from != version {
                since = now;
            }
        }
        st.snap = Some(Snap { version, sig, at: since, ids });
        save(&st);
    }
    let snap = st.snap.as_ref().ok_or("沒有道具紀錄")?;
    Ok(UpdatesOut { version: snap.version, total: snap.ids.len(), since: snap.at, records: st.records })
}

/// 讀取（必要時更新）道具紀錄；force：不管檔案有沒有變都重新讀
#[tauri::command]
pub async fn item_updates(game_dir: Option<String>, force: Option<bool>) -> Result<UpdatesOut, String> {
    tauri::async_runtime::spawn_blocking(move || check(game_dir.as_deref(), force.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let ids = vec![1002140, 1002141, 1302000, 2000000, 2000001, 5000000, 5010000];
        assert_eq!(decode_ids(&encode_ids(&ids)), ids);
    }

    #[test]
    fn builtin_list_is_sorted() {
        let ids = decode_ids(BUILTIN);
        assert!(ids.len() > 10_000);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn finds_new_items() {
        let items = vec![(1, "Item/Etc", "a".to_string()), (3, "Item/Etc", "c".to_string()), (2, "Item/Etc", "b".to_string()), (3, "Item/Etc", "c".to_string())];
        let new = added(&[1], &items);
        assert_eq!(new.iter().map(|x| x.id).collect::<Vec<_>>(), vec![2, 3]);
    }
}
