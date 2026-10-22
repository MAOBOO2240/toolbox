//! 更新道具：記下遊戲裡所有道具的編號，遊戲改版後列出新增了哪些道具。
//!
//! - 每次程式開啟（和打開工具）時檢查道具的名稱與資料檔有沒有變，有變才重新讀
//! - 只算「有名稱、而且遊戲資料裡真的有」的道具（只有名稱的還沒開放，不列）
//! - 大改版（例如 v282 → v283）後多出來的道具記成一筆；之後的小更新新增的道具也併進這一筆，
//!   直到下一次大改版才整個換成新的一筆
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

/// 紀錄格式的版本（舊格式會重新整理一次）
const STORE_VER: u32 = 2;

#[derive(Serialize, Deserialize, Default, Clone)]
struct Snap {
    version: u32,
    /// 道具名稱與資料檔的簽章（沒變就不用重新讀）
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
    /// 點裝（現金裝備）
    #[serde(default)]
    pub cash: bool,
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
    ver: u32,
    snap: Option<Snap>,
    /// 目前這次大改版的新增道具（只留一筆）
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

/// 遊戲檔簽章：道具的名稱（String）與資料（Item、裝備資料夾）的檔名、大小、修改時間；
/// 只有名稱、或只有資料的小更新也看得出來
fn signature(game: &Path) -> String {
    let data = if game.join("Data").is_dir() { game.join("Data") } else { game.to_path_buf() };
    let mut dirs: Vec<&str> = vec!["", "String", "Item", "Character", "Item/Consume", "Item/Install", "Item/Etc", "Item/Cash", "Item/Pet"];
    dirs.extend(avatar::EQUIP_DIRS);
    let mut parts: Vec<String> = Vec::new();
    for rel in dirs {
        let Ok(rd) = std::fs::read_dir(if rel.is_empty() { data.clone() } else { data.join(rel) }) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".wz") || name.ends_with(".ini")) {
                continue;
            }
            if let Ok(m) = e.metadata() {
                let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                parts.push(format!("{rel}/{name}:{}:{t}", m.len()));
            }
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    parts.sort();
    // 檔案很多，存雜湊值就好（FNV-1a）
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.join("|").bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}:{h:016x}", parts.len())
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
        .map(|(id, folder, name)| NewItem { id: *id, folder: folder.to_string(), name: name.clone(), cash: false })
        .collect();
    out.sort_by_key(|x| x.id);
    out
}

/// 依這次讀到的道具更新紀錄；cash：查這些裝備（編號, 資料夾）是不是點裝
fn apply_scan(st: &mut Store, version: u32, sig: String, items: &[(u32, &'static str, String)], cash: impl FnOnce(&[(u32, String)]) -> Vec<bool>, now: u64) {
    let mut ids: Vec<u32> = items.iter().map(|x| x.0).collect();
    ids.sort_unstable();
    ids.dedup();
    // 比較的對象：上次記錄的道具；第一次使用（或舊格式的紀錄）而且遊戲已經比內建的版本新時，用內建的 v282
    let base: Option<(u32, Vec<u32>)> = match (&st.snap, st.ver < STORE_VER) {
        (Some(s), false) => Some((s.version, s.ids.clone())),
        _ if version > BUILTIN_VERSION => Some((BUILTIN_VERSION, decode_ids(BUILTIN))),
        _ => None,
    };
    if st.ver < STORE_VER {
        // 舊格式的紀錄是只用名稱比對的（會列出沒有資料的道具），重新整理
        st.records.clear();
    }
    let mut since = st.snap.as_ref().map(|s| s.at).unwrap_or(now);
    if let Some((from, old)) = base {
        let mut new = added(&old, items);
        // 點裝：裝備的 info/cash
        let equips: Vec<(u32, String)> = new.iter().filter(|x| x.folder.starts_with("Character/")).map(|x| (x.id, x.folder.clone())).collect();
        if !equips.is_empty() {
            let flags = cash(&equips);
            let set: HashSet<u32> = equips.iter().zip(flags).filter(|(_, c)| *c).map(|((id, _), _)| *id).collect();
            for x in new.iter_mut() {
                x.cash = set.contains(&x.id);
            }
        }
        let same_major = st.records.first().is_some_and(|r| r.to == version);
        if same_major {
            // 同一次大改版之後的小更新：併進這一筆
            if !new.is_empty() {
                let r = &mut st.records[0];
                let have: HashSet<u32> = r.items.iter().map(|x| x.id).collect();
                r.items.extend(new.into_iter().filter(|x| !have.contains(&x.id)));
                r.items.sort_by_key(|x| x.id);
                r.at = now;
            }
        } else if from != version || !new.is_empty() {
            // 新的大改版（或第一次記錄之後同一版又多了道具）：整個換成新的一筆
            st.records = vec![Record { from, to: version, at: now, items: new }];
        }
        if from != version {
            since = now;
        }
    }
    st.ver = STORE_VER;
    st.snap = Some(Snap { version, sig, at: since, ids });
}

/// 檢查遊戲道具有沒有變：有新增就記進這次大改版的紀錄
fn check(hint: Option<&str>, force: bool) -> Result<UpdatesOut, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = avatar::game_dir(hint).ok_or("找不到新楓之谷安裝資料夾，請到「美容美髮」的「解析道具」頁選擇遊戲資料夾")?;
    let sig = signature(&dir);
    let mut st = load();
    let fresh = st.ver >= STORE_VER && st.snap.as_ref().is_some_and(|s| s.sig == sig && !sig.is_empty());
    if !fresh || force {
        // 遊戲檔有變（或手動重新讀取）：先丟掉之前讀好的遊戲資料，工具箱開著時遊戲更新也會讀到新的內容
        avatar::reset_ctx();
        let (version, items) = avatar::all_items(hint)?;
        if items.is_empty() {
            return Err("遊戲資料裡讀不到道具".into());
        }
        apply_scan(&mut st, version, sig, &items, |eq| avatar::cash_equips(hint, eq).unwrap_or_default(), now_ms());
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

    fn scan(st: &mut Store, version: u32, ids: &[u32], now: u64) {
        let items: Vec<(u32, &'static str, String)> = ids
            .iter()
            .map(|&id| (id, if id < 2_000_000 { "Character/Cap" } else { "Item/Consume" }, format!("道具{id}")))
            .collect();
        // 編號是奇數的帽子當作點裝
        apply_scan(st, version, format!("sig{version}-{}", ids.len()), &items, |eq| eq.iter().map(|(id, _)| id % 2 == 1).collect(), now);
    }

    #[test]
    fn major_and_minor_updates() {
        let builtin = decode_ids(BUILTIN);
        let old: Vec<u32> = builtin.iter().copied().filter(|id| (2_000_000..2_010_000).contains(id)).collect();
        assert!(!old.is_empty());
        let new_ids = [1_009_991u32, 1_009_992, 2_999_991];
        assert!(new_ids.iter().all(|id| !builtin.contains(id)));

        // 第一次使用就已經是 v283：用內建的 v282 比對
        let mut st = Store::default();
        let mut cur = old.clone();
        cur.extend_from_slice(&new_ids[..2]);
        scan(&mut st, 283, &cur, 100);
        assert_eq!(st.records.len(), 1);
        assert_eq!((st.records[0].from, st.records[0].to), (282, 283));
        assert_eq!(st.records[0].items.iter().map(|x| x.id).collect::<Vec<_>>(), vec![1_009_991, 1_009_992]);
        assert!(st.records[0].items[0].cash && !st.records[0].items[1].cash);

        // v283 的小更新：併進同一筆
        cur.push(new_ids[2]);
        scan(&mut st, 283, &cur, 200);
        assert_eq!(st.records.len(), 1);
        assert_eq!(st.records[0].items.len(), 3);
        assert_eq!(st.records[0].at, 200);

        // 沒有變化：不動
        scan(&mut st, 283, &cur, 300);
        assert_eq!(st.records[0].at, 200);

        // 大改版 v284：整個換掉
        cur.push(4_999_991);
        scan(&mut st, 284, &cur, 400);
        assert_eq!(st.records.len(), 1);
        assert_eq!((st.records[0].from, st.records[0].to), (283, 284));
        assert_eq!(st.records[0].items.iter().map(|x| x.id).collect::<Vec<_>>(), vec![4_999_991]);
    }

    #[test]
    fn first_use_on_builtin_version_records_nothing() {
        let mut st = Store::default();
        scan(&mut st, 282, &[2_000_000, 2_000_001], 1);
        assert!(st.records.is_empty());
        // 同一版的小更新多了道具
        scan(&mut st, 282, &[2_000_000, 2_000_001, 2_999_992], 2);
        assert_eq!(st.records.len(), 1);
        assert_eq!((st.records[0].from, st.records[0].to), (282, 282));
    }

    #[test]
    fn old_format_is_rebuilt() {
        let mut st = Store { ver: 0, snap: Some(Snap { version: 283, sig: "x".into(), at: 5, ids: vec![2_000_000] }), records: vec![Record { from: 282, to: 283, at: 5, items: vec![] }] };
        scan(&mut st, 283, &[2_000_000, 2_999_993], 9);
        assert_eq!(st.ver, STORE_VER);
        assert_eq!(st.records.len(), 1);
        assert_eq!((st.records[0].from, st.records[0].to), (282, 283));
        assert_eq!(st.records[0].items.iter().map(|x| x.id).collect::<Vec<_>>(), vec![2_999_993]);
    }

    #[test]
    fn signature_sees_name_and_data_files() {
        let root = std::env::temp_dir().join(format!("itemup-sig-{}-{}", std::process::id(), now_ms()));
        let data = root.join("Data");
        for d in ["String", "Item/Consume", "Character/Cap", "Map"] {
            std::fs::create_dir_all(data.join(d)).unwrap();
        }
        std::fs::write(data.join("String/String_000.wz"), b"a").unwrap();
        std::fs::write(data.join("Item/Consume/Consume_000.wz"), b"b").unwrap();
        std::fs::write(data.join("Character/Cap/Cap_000.wz"), b"c").unwrap();
        let s1 = signature(&root);
        assert!(s1.starts_with("3:"), "{s1}");
        // 只有資料檔變大（小更新只加資料）
        std::fs::write(data.join("Item/Consume/Consume_000.wz"), b"bb").unwrap();
        let s2 = signature(&root);
        assert_ne!(s1, s2);
        // 裝備多一個檔案
        std::fs::write(data.join("Character/Cap/Cap_001.wz"), b"d").unwrap();
        let s3 = signature(&root);
        assert_ne!(s2, s3);
        // 不相關的資料夾（地圖）不算
        std::fs::write(data.join("Map/Map_000.wz"), b"e").unwrap();
        assert_eq!(signature(&root), s3);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(signature(&root), "");
    }

    #[test]
    fn finds_new_items() {
        let items = vec![(1, "Item/Etc", "a".to_string()), (3, "Item/Etc", "c".to_string()), (2, "Item/Etc", "b".to_string()), (3, "Item/Etc", "c".to_string())];
        let new = added(&[1], &items);
        assert_eq!(new.iter().map(|x| x.id).collect::<Vec<_>>(), vec![2, 3]);
    }
}
