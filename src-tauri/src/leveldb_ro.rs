//! 唯讀的 LevelDB 讀取器：把一個 LevelDB 資料夾目前的內容（鍵 → 值）整份讀出來。
//!
//! 用途：Chrome 裡每個擴充功能的存放區（chrome.storage.local）就是一個 LevelDB 資料夾；
//! 讀篡改猴的那一個，就能把使用者在 Chrome 的篡改猴裡的腳本帶過來（見 web/tm.rs 的「Chrome 的篡改猴」）。
//!
//! 只開檔案來讀：不寫、不鎖、不刪、不複製任何東西，Chrome 開著也可以讀（它的 LOCK 檔完全不碰）。
//! 讀的當下資料庫如果剛好在變動（Chrome 正在整理檔案），會發現並重讀，不會拿到少一截的內容。
//!
//! 格式（LevelDB 公開的文件）：
//! - CURRENT：目前的 MANIFEST 檔名
//! - MANIFEST-xxxxxx：哪些資料表檔是有效的、從哪一個記錄檔開始的變動還沒寫進資料表
//! - xxxxxx.ldb（舊版是 .sst）：排序好的資料表，一塊一塊存，可能用 snappy 壓縮
//! - xxxxxx.log：還沒寫進資料表的變動
//!
//! 同一個鍵可能出現很多次（每次寫入都帶一個遞增的序號）：取序號最大的那一筆；那一筆是「刪除」就當作沒有。

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::time::Duration;

/// 全部檔案加起來超過這麼大就不讀（正常的擴充功能存放區不會這麼大）
const MAX_TOTAL: u64 = 768 << 20;
/// 一塊資料解壓縮後最大多少
const MAX_BLOCK: usize = 256 << 20;

/* ---------------- 小工具 ---------------- */

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC: [u32; 256] = crc_table();

/// CRC-32C（Castagnoli）
fn crc32c(parts: &[&[u8]]) -> u32 {
    let mut c = !0u32;
    for p in parts {
        for &b in *p {
            c = CRC[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    !c
}

/// LevelDB 存的檢查碼是轉過的（避免資料裡剛好含有自己的檢查碼）
fn mask(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(0xa282_ead8)
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// 長度不固定的整數（每個位元組 7 位，最高位是「後面還有」）
fn varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let (mut r, mut shift) = (0u64, 0u32);
    loop {
        let byte = *b.get(*pos)?;
        *pos += 1;
        r |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(r);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// 「長度＋內容」的一段
fn chunk<'a>(b: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let n = usize::try_from(varint(b, pos)?).ok()?;
    let s = b.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    Some(s)
}

/// snappy 解壓縮（整塊的那種格式）
fn snappy(src: &[u8]) -> Option<Vec<u8>> {
    let mut p = 0;
    let n = usize::try_from(varint(src, &mut p)?).ok()?;
    if n > MAX_BLOCK {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(n);
    while p < src.len() {
        let tag = src[p];
        p += 1;
        let (len, off) = match tag & 3 {
            // 照抄一段
            0 => {
                let mut len = (tag >> 2) as usize;
                if len >= 60 {
                    let nb = len - 59;
                    let b = src.get(p..p + nb)?;
                    len = b.iter().rev().fold(0usize, |a, x| (a << 8) | *x as usize);
                    p += nb;
                }
                len += 1;
                let lit = src.get(p..p.checked_add(len)?)?;
                if out.len() + len > n {
                    return None;
                }
                out.extend_from_slice(lit);
                p += len;
                continue;
            }
            // 重複前面出現過的一段（三種寫法）
            1 => {
                let len = ((tag >> 2) & 7) as usize + 4;
                let off = (((tag >> 5) as usize) << 8) | *src.get(p)? as usize;
                p += 1;
                (len, off)
            }
            2 => {
                let b = src.get(p..p + 2)?;
                p += 2;
                ((tag >> 2) as usize + 1, u16::from_le_bytes([b[0], b[1]]) as usize)
            }
            _ => {
                let b = src.get(p..p + 4)?;
                p += 4;
                ((tag >> 2) as usize + 1, u32le(b) as usize)
            }
        };
        if off == 0 || off > out.len() || out.len() + len > n {
            return None;
        }
        // 可能跟自己重疊（一個一個位元組抄）
        let start = out.len() - off;
        for i in 0..len {
            let b = out[start + i];
            out.push(b);
        }
    }
    (out.len() == n).then_some(out)
}

/* ---------------- 記錄檔（.log 和 MANIFEST 都是這個格式） ---------------- */

/// 一筆一筆讀出來。檔案切成 32768 位元組的區塊，一筆可以跨好幾塊；尾巴沒寫完、檢查碼不對的不算
fn log_records(data: &[u8]) -> Vec<Vec<u8>> {
    const BLOCK: usize = 32768;
    let mut out = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    let mut pos = 0;
    while pos < data.len() {
        let left = BLOCK - pos % BLOCK;
        // 區塊尾巴放不下標頭：補零的，跳到下一塊
        if left < 7 {
            pos += left;
            continue;
        }
        if pos + 7 > data.len() {
            break;
        }
        let crc = u32le(&data[pos..]);
        let len = u16::from_le_bytes([data[pos + 4], data[pos + 5]]) as usize;
        let ty = data[pos + 6];
        // 壞掉的一段（長度不合理、檢查碼不對、空白）：這一塊剩下的不要，寫到一半的那一筆也作廢
        let bad = len > left - 7 || pos + 7 + len > data.len() || (ty == 0 && len == 0) || mask(crc32c(&[&[ty], &data[pos + 7..pos + 7 + len]])) != crc;
        if bad {
            cur = None;
            pos += left;
            continue;
        }
        let payload = &data[pos + 7..pos + 7 + len];
        pos += 7 + len;
        match ty {
            // 完整的一筆
            1 => {
                cur = None;
                out.push(payload.to_vec());
            }
            // 第一段、中間、最後一段
            2 => cur = Some(payload.to_vec()),
            3 => {
                if let Some(c) = cur.as_mut() {
                    c.extend_from_slice(payload);
                }
            }
            4 => {
                if let Some(mut c) = cur.take() {
                    c.extend_from_slice(payload);
                    out.push(c);
                }
            }
            _ => cur = None,
        }
    }
    out
}

/// MANIFEST 的內容：有效的資料表檔（層, 編號）、從哪個記錄檔開始還沒寫進資料表
#[derive(Default)]
struct Version {
    tables: BTreeSet<(u64, u64)>,
    log: u64,
    prev_log: u64,
}

fn parse_manifest(data: &[u8]) -> Option<Version> {
    let mut v = Version::default();
    for rec in log_records(data) {
        let mut p = 0;
        while p < rec.len() {
            match varint(&rec, &mut p)? {
                // 排序方式的名稱（這裡不靠順序，不用管）
                1 => {
                    chunk(&rec, &mut p)?;
                }
                2 => v.log = varint(&rec, &mut p)?,
                // 下一個檔案編號、最後的序號
                3 | 4 => {
                    varint(&rec, &mut p)?;
                }
                // 整理到哪裡的記號
                5 => {
                    varint(&rec, &mut p)?;
                    chunk(&rec, &mut p)?;
                }
                // 不要了的資料表
                6 => {
                    let level = varint(&rec, &mut p)?;
                    let num = varint(&rec, &mut p)?;
                    v.tables.remove(&(level, num));
                }
                // 新的資料表：層、編號、大小、最小的鍵、最大的鍵
                7 => {
                    let level = varint(&rec, &mut p)?;
                    let num = varint(&rec, &mut p)?;
                    varint(&rec, &mut p)?;
                    chunk(&rec, &mut p)?;
                    chunk(&rec, &mut p)?;
                    v.tables.insert((level, num));
                }
                9 => v.prev_log = varint(&rec, &mut p)?,
                _ => return None,
            }
        }
    }
    Some(v)
}

/* ---------------- 資料表檔 ---------------- */

/// 讀一塊（位置, 大小）：後面跟著 1 個位元組的壓縮方式和 4 個位元組的檢查碼
fn read_block(file: &[u8], off: u64, size: u64) -> Option<Vec<u8>> {
    let off = usize::try_from(off).ok()?;
    let end = off.checked_add(usize::try_from(size).ok()?)?;
    let tail = file.get(end..end.checked_add(5)?)?;
    let raw = file.get(off..end)?;
    if mask(crc32c(&[raw, &tail[..1]])) != u32le(&tail[1..]) {
        return None;
    }
    match tail[0] {
        0 => Some(raw.to_vec()),
        1 => snappy(raw),
        _ => None,
    }
}

/// 一塊裡的每一筆（鍵, 值）。鍵只存跟前一筆不一樣的部分；最後面是「重新開始的位置」清單
fn block_entries(block: &[u8], f: &mut dyn FnMut(&[u8], &[u8]) -> Option<()>) -> Option<()> {
    let n = u32le(block.get(block.len().checked_sub(4)?..)?) as usize;
    let end = block.len().checked_sub(n.checked_mul(4)?.checked_add(4)?)?;
    let mut p = 0;
    let mut key: Vec<u8> = Vec::new();
    while p < end {
        let shared = usize::try_from(varint(block, &mut p)?).ok()?;
        let rest = usize::try_from(varint(block, &mut p)?).ok()?;
        let vlen = usize::try_from(varint(block, &mut p)?).ok()?;
        if shared > key.len() {
            return None;
        }
        let k_end = p.checked_add(rest)?;
        let v_end = k_end.checked_add(vlen)?;
        if v_end > end {
            return None;
        }
        key.truncate(shared);
        key.extend_from_slice(&block[p..k_end]);
        f(&key, &block[k_end..v_end])?;
        p = v_end;
    }
    Some(())
}

/// 一個資料表檔裡的每一筆（含序號的鍵, 值）
fn table_entries(file: &[u8], f: &mut dyn FnMut(&[u8], &[u8]) -> Option<()>) -> Option<()> {
    const MAGIC: [u8; 8] = [0x57, 0xfb, 0x80, 0x8b, 0x24, 0x75, 0x47, 0xdb];
    let foot = file.get(file.len().checked_sub(48)?..)?;
    if foot[40..] != MAGIC {
        return None;
    }
    let mut p = 0;
    // 前兩個數字是附加資訊的索引（用不到），後兩個是資料的索引
    varint(foot, &mut p)?;
    varint(foot, &mut p)?;
    let (ioff, isize) = (varint(foot, &mut p)?, varint(foot, &mut p)?);
    let index = read_block(file, ioff, isize)?;
    block_entries(&index, &mut |_k, v| {
        let mut q = 0;
        let (off, size) = (varint(v, &mut q)?, varint(v, &mut q)?);
        let data = read_block(file, off, size)?;
        block_entries(&data, &mut *f)
    })
}

/* ---------------- 整個資料庫 ---------------- */

type Seen = HashMap<Vec<u8>, (u64, Option<Vec<u8>>)>;

fn put(map: &mut Seen, key: &[u8], seq: u64, val: Option<&[u8]>) {
    match map.get_mut(key) {
        Some(e) if e.0 > seq => {}
        Some(e) => *e = (seq, val.map(<[u8]>::to_vec)),
        None => {
            map.insert(key.to_vec(), (seq, val.map(<[u8]>::to_vec)));
        }
    }
}

/// 檔名開頭的編號（000123.log → 123）
fn file_num(name: &str, ext: &str) -> Option<u64> {
    let stem = name.strip_suffix(ext)?;
    (!stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())).then(|| stem.parse().ok())?
}

/// 目前的 MANIFEST 檔名、資料夾裡的記錄檔編號（讀之前、讀之後各看一次：不一樣就是讀的當中變動了）
fn layout(dir: &Path) -> Result<(String, u64, Vec<u64>), String> {
    let cur = std::fs::read_to_string(dir.join("CURRENT")).map_err(|e| format!("CURRENT：{e}"))?;
    let name = cur.trim().to_string();
    if !name.starts_with("MANIFEST-") || name.contains(['/', '\\']) {
        return Err("CURRENT 的內容不對".into());
    }
    let len = std::fs::metadata(dir.join(&name)).map_err(|e| format!("{name}：{e}"))?.len();
    let mut logs: Vec<u64> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| file_num(&e.file_name().to_string_lossy(), ".log"))
        .collect();
    logs.sort_unstable();
    Ok((name, len, logs))
}

/// 讀一次。Ok(None) = 讀的當中資料庫變動了（要重讀）
fn read_once(dir: &Path) -> Result<Option<HashMap<Vec<u8>, Vec<u8>>>, String> {
    let before = layout(dir)?;
    let manifest = std::fs::read(dir.join(&before.0)).map_err(|e| format!("{}：{e}", before.0))?;
    if manifest.len() as u64 != before.1 {
        return Ok(None);
    }
    let ver = parse_manifest(&manifest).ok_or("MANIFEST 看不懂")?;
    let mut total = manifest.len() as u64;
    let mut map: Seen = HashMap::new();
    // 資料表：鍵的最後 8 個位元組是（序號 << 8 | 種類），種類 1 = 值、0 = 刪除
    for (_, num) in &ver.tables {
        let file = [format!("{num:06}.ldb"), format!("{num:06}.sst")]
            .iter()
            .find_map(|n| std::fs::read(dir.join(n)).ok())
            .ok_or_else(|| format!("資料表 {num:06} 讀不到"))?;
        total += file.len() as u64;
        if total > MAX_TOTAL {
            return Err("資料庫太大".into());
        }
        table_entries(&file, &mut |k, v| {
            let cut = k.len().checked_sub(8)?;
            let tail = u64::from_le_bytes(k[cut..].try_into().ok()?);
            match tail & 0xff {
                1 => put(&mut map, &k[..cut], tail >> 8, Some(v)),
                0 => put(&mut map, &k[..cut], tail >> 8, None),
                _ => return None,
            }
            Some(())
        })
        .ok_or_else(|| format!("資料表 {num:06} 看不懂"))?;
    }
    // 記錄檔：還沒寫進資料表的變動（一筆 = 序號、幾個動作、然後每個動作：種類、鍵、值）
    for num in before.2.iter().filter(|n| **n >= ver.log || (ver.prev_log != 0 && **n == ver.prev_log)) {
        let file = std::fs::read(dir.join(format!("{num:06}.log"))).map_err(|e| format!("記錄檔 {num:06}：{e}"))?;
        total += file.len() as u64;
        if total > MAX_TOTAL {
            return Err("資料庫太大".into());
        }
        for rec in log_records(&file) {
            let batch = (|| {
                let seq = u64::from_le_bytes(rec.get(..8)?.try_into().ok()?);
                let count = u32le(rec.get(8..12)?) as u64;
                let mut p = 12;
                for i in 0..count {
                    let ty = *rec.get(p)?;
                    p += 1;
                    let key = chunk(&rec, &mut p)?;
                    match ty {
                        1 => {
                            let val = chunk(&rec, &mut p)?;
                            put(&mut map, key, seq + i, Some(val));
                        }
                        0 => put(&mut map, key, seq + i, None),
                        _ => return None,
                    }
                }
                Some(())
            })();
            if batch.is_none() {
                return Err(format!("記錄檔 {num:06} 看不懂"));
            }
        }
    }
    // 讀的當中有沒有變動（Chrome 把記錄檔寫進資料表、整理檔案時，MANIFEST 會變長、記錄檔會換）
    if layout(dir)? != before {
        return Ok(None);
    }
    Ok(Some(map.into_iter().filter_map(|(k, (_, v))| v.map(|v| (k, v))).collect()))
}

/// 把整個資料庫目前的內容讀出來（鍵 → 值）。讀的當中有變動、或暫時讀不到檔案，會等一下重讀幾次
pub fn read_all(dir: &Path) -> Result<HashMap<Vec<u8>, Vec<u8>>, String> {
    // 根本沒有這個資料庫：不用重試
    if !dir.join("CURRENT").is_file() {
        return Err("沒有這個資料庫".into());
    }
    let mut last = String::new();
    for attempt in 0..6u64 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(150 * attempt));
        }
        match read_once(dir) {
            Ok(Some(map)) => return Ok(map),
            Ok(None) => last = "讀的時候資料庫一直在變動".into(),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_and_varint() {
        // CRC-32C 的標準測試值
        assert_eq!(crc32c(&[b"123456789"]), 0xe306_9283);
        assert_eq!(crc32c(&[b"1234", b"56789"]), 0xe306_9283);
        let mut p = 0;
        assert_eq!(varint(&[0xac, 0x02, 0x7f], &mut p), Some(300));
        assert_eq!((varint(&[0xac, 0x02, 0x7f], &mut p), p), (Some(127), 3));
        assert_eq!(varint(&[0x80], &mut 0), None);
        assert_eq!(file_num("000123.log", ".log"), Some(123));
        assert_eq!((file_num("LOG", ".log"), file_num("x1.log", ".log"), file_num(".log", ".log")), (None, None, None));
    }

    #[test]
    fn snappy_blocks() {
        // 「abcabcabcabc」：照抄 abc，再重複前面 3 個位元組共 9 個
        assert_eq!(snappy(&[12, 0x08, b'a', b'b', b'c', 0x15, 0x03]).as_deref(), Some(&b"abcabcabcabc"[..]));
        // 長度對不上、往回超過開頭：不接受
        assert_eq!(snappy(&[5, 0x08, b'a', b'b', b'c']), None);
        assert_eq!(snappy(&[8, 0x00, b'a', 0x15, 0x09]), None);
        assert_eq!(snappy(&[0]).as_deref(), Some(&b""[..]));
    }

    /// 用 LevelDB 的官方實作產生的一個小資料庫（testdata/leveldb.bin：三個資料表檔、一個記錄檔，
    /// 有壓縮的和壓不動的資料、覆寫、刪除、一筆跨好幾個區塊的大資料），讀出來要跟當初寫進去的一模一樣
    #[test]
    fn reads_a_real_database() {
        let pack = include_bytes!("testdata/leveldb.bin");
        let dir = std::env::temp_dir().join(format!("mb-ldb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 打包檔：每個檔案是（檔名長度 2 位元組、檔名、內容長度 4 位元組、內容）
        let mut p = 0;
        while p < pack.len() {
            let n = u16::from_le_bytes([pack[p], pack[p + 1]]) as usize;
            let name = std::str::from_utf8(&pack[p + 2..p + 2 + n]).unwrap();
            let len = u32le(&pack[p + 2 + n..]) as usize;
            std::fs::write(dir.join(name), &pack[p + 6 + n..p + 6 + n + len]).unwrap();
            p += 6 + n + len;
        }
        // 當初寫進去的內容（跟產生資料庫時同樣的規則）
        let noise = |n: usize, seed: u32| -> Vec<u8> {
            let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x & 0xff) as u8
                })
                .collect()
        };
        let key = |i: usize| format!("k{i:04}").into_bytes();
        let mut want: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for i in 0..400 {
            want.insert(key(i), format!("v{i}:{}", "ab".repeat(i % 50)).into_bytes());
        }
        for i in 0..12u32 {
            want.insert(format!("r{i:02}").into_bytes(), noise(3000, i));
        }
        for i in (0..400).step_by(3) {
            want.remove(&key(i));
        }
        for i in (0..400).step_by(4) {
            want.insert(key(i), format!("n{i}").into_bytes());
        }
        want.insert("中文".as_bytes().to_vec(), "值".as_bytes().to_vec());
        want.insert(b"b1".to_vec(), b"batch".to_vec());
        want.remove(&key(1));
        want.insert(b"b2".to_vec(), Vec::new());
        want.insert(b"big".to_vec(), vec![b'x'; 70000]);
        let got = read_all(&dir).unwrap();
        assert_eq!(got.len(), 315);
        assert!(got == want);

        // 多了別的檔案（Chrome 的 LOCK、LOG）不影響
        std::fs::write(dir.join("LOCK"), b"").unwrap();
        std::fs::write(dir.join("LOG"), b"whatever").unwrap();
        assert!(read_once(&dir).unwrap().is_some_and(|m| m == want));
        // 記錄檔尾巴多了寫到一半的東西：前面完整的照樣讀得到
        let log = dir.join("000010.log");
        let good = std::fs::read(&log).unwrap();
        let mut torn = good.clone();
        torn.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x40, 0x00, 0x01, 1, 2, 3]);
        std::fs::write(&log, &torn).unwrap();
        assert!(read_once(&dir).unwrap().is_some_and(|m| m == want));
        std::fs::write(&log, &good).unwrap();
        // 資料表壞了（中間改一個位元組）、少了一個檔、CURRENT 不對：回報讀不到，不會當掉、也不會給錯的內容
        let table = dir.join("000005.ldb");
        let orig = std::fs::read(&table).unwrap();
        let mut bad = orig.clone();
        bad[1000] ^= 0xff;
        std::fs::write(&table, &bad).unwrap();
        assert!(read_once(&dir).is_err());
        std::fs::remove_file(&table).unwrap();
        assert!(read_once(&dir).is_err());
        std::fs::write(&table, &orig).unwrap();
        assert!(read_once(&dir).unwrap().is_some());
        std::fs::write(dir.join("CURRENT"), b"../MANIFEST-000008\n").unwrap();
        assert!(read_once(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_all(&dir).is_err());
    }
}
