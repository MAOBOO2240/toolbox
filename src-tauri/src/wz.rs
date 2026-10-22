//! 新楓之谷 WZ 檔讀取（唯讀）。
//!
//! 支援新版的分割格式：`Data/<分類>/<分類>_000.wz ...`，圖片存放在 `_Canvas` 子資料夾，
//! 由屬性 `_outlink` 指向。字串使用零金鑰（不需額外解密），版本號自動偵測。

use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/* ---------------- 低階讀取 ---------------- */

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(d: &'a [u8], p: usize) -> Self {
        Self { d, p }
    }
    fn need(&self, n: usize) -> Result<(), String> {
        // 遊戲檔損壞時位置可能是亂的：用不會溢位的加法判斷
        if self.p.checked_add(n).map_or(true, |e| e > self.d.len()) {
            Err(format!("讀取超出範圍 @{}", self.p))
        } else {
            Ok(())
        }
    }
    fn u8(&mut self) -> Result<u8, String> {
        self.need(1)?;
        let v = self.d[self.p];
        self.p += 1;
        Ok(v)
    }
    fn i8(&mut self) -> Result<i8, String> {
        Ok(self.u8()? as i8)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.d[self.p], self.d[self.p + 1]]);
        self.p += 2;
        Ok(v)
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.need(4)?;
        let v = u32::from_le_bytes(self.d[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        Ok(v)
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn i64(&mut self) -> Result<i64, String> {
        self.need(8)?;
        let v = i64::from_le_bytes(self.d[self.p..self.p + 8].try_into().unwrap());
        self.p += 8;
        Ok(v)
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.i64()? as u64))
    }
    fn cint(&mut self) -> Result<i32, String> {
        let b = self.i8()?;
        if b == -128 { self.i32() } else { Ok(b as i32) }
    }
    fn clong(&mut self) -> Result<i64, String> {
        let b = self.i8()?;
        if b == -128 { self.i64() } else { Ok(b as i64) }
    }
    /// WZ 字串（零金鑰：只需與遞增遮罩 XOR）
    fn wzstr(&mut self) -> Result<String, String> {
        let n = self.i8()?;
        if n == 0 {
            return Ok(String::new());
        }
        if n > 0 {
            let len = if n == 127 { self.i32()? as usize } else { n as usize };
            self.need(len.saturating_mul(2))?;
            let mut mask: u16 = 0xAAAA;
            let mut units = Vec::with_capacity(len);
            for _ in 0..len {
                units.push(self.u16()? ^ mask);
                mask = mask.wrapping_add(1);
            }
            Ok(String::from_utf16_lossy(&units))
        } else {
            let len = if n == -128 { self.i32()? as usize } else { (-(n as i32)) as usize };
            self.need(len)?;
            let mut mask: u8 = 0xAA;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(self.u8()? ^ mask);
                mask = mask.wrapping_add(1);
            }
            // ASCII 範圍以外的位元組視為 Latin-1
            Ok(out.into_iter().map(|b| b as char).collect())
        }
    }
}

fn version_hash(v: u32) -> u32 {
    let mut h: u32 = 0;
    for c in v.to_string().bytes() {
        h = h.wrapping_mul(32).wrapping_add(c as u32 + 1);
    }
    h
}

fn enc_version(h: u32) -> u16 {
    (0xFF ^ ((h >> 24) & 0xFF) ^ ((h >> 16) & 0xFF) ^ ((h >> 8) & 0xFF) ^ (h & 0xFF)) as u16
}

/* ---------------- 檔案與目錄 ---------------- */

#[derive(Clone, Debug)]
pub struct Entry {
    pub is_dir: bool,
    pub name: String,
    pub offset: usize,
}

pub struct WzFile {
    map: Mmap,
    fstart: usize,
    hash: u32,
    pub version: u32,
}

impl WzFile {
    pub fn open(path: &Path, version_hint: Option<u32>) -> Result<Self, String> {
        let f = File::open(path).map_err(|e| format!("無法開啟 {}：{e}", path.display()))?;
        // SAFETY: 唯讀對應，遊戲執行中更新檔案的情況下最壞只會讀到錯誤資料
        let map = unsafe { Mmap::map(&f) }.map_err(|e| e.to_string())?;
        if map.len() < 16 || &map[0..4] != b"PKG1" {
            return Err(format!("{} 不是 WZ 檔", path.display()));
        }
        let fstart = u32::from_le_bytes(map[12..16].try_into().unwrap()) as usize;
        let mut wz = WzFile { map, fstart, hash: 0, version: version_hint.unwrap_or(0) };
        if wz.fstart + 2 >= wz.map.len() {
            return Ok(wz); // 空檔（只有標頭）
        }
        let enc = u16::from_le_bytes([wz.map[fstart], wz.map[fstart + 1]]);
        let mut candidates: Vec<u32> = Vec::new();
        if let Some(v) = version_hint {
            candidates.push(v);
        }
        candidates.extend(1..3000);
        for v in candidates {
            let h = version_hash(v);
            if enc_version(h) != enc {
                continue;
            }
            wz.hash = h;
            wz.version = v;
            if wz.looks_valid() {
                return Ok(wz);
            }
        }
        Err(format!("{} 無法判斷版本", path.display()))
    }

    pub fn data(&self) -> &[u8] {
        &self.map
    }

    pub fn version_hash(&self) -> u32 {
        self.hash
    }

    fn looks_valid(&self) -> bool {
        match self.read_dir(self.fstart + 2) {
            Ok(ents) if !ents.is_empty() => ents.iter().take(64).all(|e| {
                e.offset < self.map.len() && (e.is_dir || matches!(self.map[e.offset], 0x73 | 0x1B))
            }),
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn offset(&self, r: &mut Reader) -> Result<usize, String> {
        let pos = r.p as u32;
        let fstart = self.fstart as u32;
        let mut o = (pos.wrapping_sub(fstart)) ^ 0xFFFF_FFFF;
        o = o.wrapping_mul(self.hash);
        o = o.wrapping_sub(0x581C_3F6D);
        o = o.rotate_left(o & 0x1F);
        o ^= r.u32()?;
        o = o.wrapping_add(fstart.wrapping_mul(2));
        Ok(o as usize)
    }

    pub fn read_dir(&self, pos: usize) -> Result<Vec<Entry>, String> {
        let d = &self.map[..];
        let mut r = Reader::new(d, pos);
        let n = r.cint()?;
        if !(0..=1_000_000).contains(&n) {
            return Err("目錄數量異常".into());
        }
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let t = r.u8()?;
            let (kind, name) = match t {
                1 => {
                    r.i32()?;
                    r.i16()?;
                    r.u32()?;
                    continue;
                }
                2 => {
                    let so = r.i32()? as usize;
                    let mut r2 = Reader::new(d, self.fstart + 1 + so);
                    let k = r2.u8()?;
                    (k, r2.wzstr()?)
                }
                3 | 4 => (t, r.wzstr()?),
                _ => return Err(format!("目錄類型異常 {t}")),
            };
            r.cint()?; // 大小
            r.cint()?; // 檢查碼
            let offset = self.offset(&mut r)?;
            out.push(Entry { is_dir: kind == 3, name, offset });
        }
        Ok(out)
    }

    pub fn root(&self) -> Result<Vec<Entry>, String> {
        if self.fstart + 2 >= self.map.len() {
            return Ok(Vec::new());
        }
        self.read_dir(self.fstart + 2)
    }
}

/* ---------------- 屬性樹 ---------------- */

#[derive(Clone, Debug)]
pub struct CanvasRef {
    pub w: u32,
    pub h: u32,
    pub fmt: i32,
    pub data_off: usize,
    pub data_len: usize,
}

#[derive(Clone, Debug)]
pub enum Node {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Vec(i32, i32),
    Uol(String),
    Canvas(CanvasRef, Vec<(String, Node)>),
    Props(Vec<(String, Node)>),
    Other,
}

impl Node {
    pub fn children(&self) -> &[(String, Node)] {
        match self {
            Node::Props(c) | Node::Canvas(_, c) => c,
            _ => &[],
        }
    }
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.children().iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// 以 `a/b/c` 路徑取得子節點
    pub fn path(&self, p: &str) -> Option<&Node> {
        let mut cur = self;
        for seg in p.split('/').filter(|s| !s.is_empty()) {
            cur = cur.get(seg)?;
        }
        Some(cur)
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Node::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Node::Int(v) => Some(*v),
            Node::Float(f) => Some(*f as i64),
            Node::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn as_vec(&self) -> Option<(i32, i32)> {
        if let Node::Vec(x, y) = self { Some((*x, *y)) } else { None }
    }
}

pub fn parse_img(file: &WzFile, off: usize) -> Result<Node, String> {
    let d = file.data();
    let mut r = Reader::new(d, off);
    let head = str_block(&mut r, d, off)?;
    if head != "Property" {
        return Err(format!("img 開頭不是 Property：{head}"));
    }
    Ok(Node::Props(props(&mut r, d, off)?))
}

/// 只讀 img 最上層的某一個屬性（例如 info），不解析其他動作，速度快很多
pub fn parse_img_child(file: &WzFile, off: usize, key: &str) -> Result<Option<Node>, String> {
    let d = file.data();
    let mut r = Reader::new(d, off);
    if str_block(&mut r, d, off)? != "Property" {
        return Err("img 開頭不是 Property".into());
    }
    r.u16()?;
    let n = r.cint()?;
    for _ in 0..n.clamp(0, 100_000) {
        let name = str_block(&mut r, d, off)?;
        match r.u8()? {
            0 => {}
            2 | 11 => {
                r.i16()?;
            }
            3 | 19 => {
                r.cint()?;
            }
            20 => {
                r.clong()?;
            }
            4 => {
                if r.u8()? == 0x80 {
                    r.f32()?;
                }
            }
            5 => {
                r.f64()?;
            }
            8 => {
                str_block(&mut r, d, off)?;
            }
            9 => {
                let len = r.u32()? as usize;
                let end = r.p + len;
                if name == key {
                    return Ok(Some(extended(&mut r, d, off)?));
                }
                r.p = end;
            }
            t => return Err(format!("屬性類型異常 {t}")),
        }
    }
    Ok(None)
}

/// 只列出 img 最上層的屬性名稱，不解析內容（例如道具 img 裡有哪些道具編號）
pub fn img_keys(file: &WzFile, off: usize) -> Result<Vec<String>, String> {
    let d = file.data();
    let mut r = Reader::new(d, off);
    if str_block(&mut r, d, off)? != "Property" {
        return Err("img 開頭不是 Property".into());
    }
    r.u16()?;
    let n = r.cint()?;
    let mut out = Vec::with_capacity(n.clamp(0, 500_000) as usize);
    for _ in 0..n.clamp(0, 500_000) {
        let name = str_block(&mut r, d, off)?;
        match r.u8()? {
            0 => {}
            2 | 11 => {
                r.i16()?;
            }
            3 | 19 => {
                r.cint()?;
            }
            20 => {
                r.clong()?;
            }
            4 => {
                if r.u8()? == 0x80 {
                    r.f32()?;
                }
            }
            5 => {
                r.f64()?;
            }
            8 => {
                str_block(&mut r, d, off)?;
            }
            9 => {
                let len = r.u32()? as usize;
                r.p = r.p.saturating_add(len);
            }
            t => return Err(format!("屬性類型異常 {t}")),
        }
        out.push(name);
    }
    Ok(out)
}

fn str_block(r: &mut Reader, d: &[u8], img: usize) -> Result<String, String> {
    match r.u8()? {
        0x00 | 0x73 => r.wzstr(),
        0x01 | 0x1B => {
            let so = r.i32()? as usize;
            Reader::new(d, img + so).wzstr()
        }
        t => Err(format!("字串區塊類型異常 {t:#x}")),
    }
}

fn props(r: &mut Reader, d: &[u8], img: usize) -> Result<Vec<(String, Node)>, String> {
    r.u16()?;
    let n = r.cint()?;
    if !(0..=500_000).contains(&n) {
        return Err("屬性數量異常".into());
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let name = str_block(r, d, img)?;
        let t = r.u8()?;
        let v = match t {
            0 => Node::Null,
            2 | 11 => Node::Int(r.i16()? as i64),
            3 | 19 => Node::Int(r.cint()? as i64),
            20 => Node::Int(r.clong()?),
            4 => Node::Float(if r.u8()? == 0x80 { r.f32()? as f64 } else { 0.0 }),
            5 => Node::Float(r.f64()?),
            8 => Node::Str(str_block(r, d, img)?),
            9 => {
                let len = r.u32()? as usize;
                let end = r.p + len;
                let v = extended(r, d, img)?;
                r.p = end;
                v
            }
            _ => return Err(format!("屬性類型異常 {t}（{name}）")),
        };
        out.push((name, v));
    }
    Ok(out)
}

fn extended(r: &mut Reader, d: &[u8], img: usize) -> Result<Node, String> {
    let t = r.u8()?;
    let name = match t {
        0x01 | 0x1B => {
            let so = r.i32()? as usize;
            Reader::new(d, img + so).wzstr()?
        }
        _ => r.wzstr()?,
    };
    match name.as_str() {
        "Property" => Ok(Node::Props(props(r, d, img)?)),
        "Canvas" => {
            r.u8()?;
            let p = if r.u8()? == 1 { props(r, d, img)? } else { Vec::new() };
            let w = r.cint()? as u32;
            let h = r.cint()? as u32;
            let f = r.cint()?;
            let f2 = r.u8()? as i32;
            r.u32()?;
            let len = r.i32()?.max(1) as usize;
            r.u8()?;
            let c = CanvasRef { w, h, fmt: f + f2, data_off: r.p, data_len: len - 1 };
            r.p += len - 1;
            Ok(Node::Canvas(c, p))
        }
        "Shape2D#Vector2D" => Ok(Node::Vec(r.cint()?, r.cint()?)),
        "Shape2D#Convex2D" => {
            let n = r.cint()?;
            let mut v = Vec::new();
            for i in 0..n {
                v.push((i.to_string(), extended(r, d, img)?));
            }
            Ok(Node::Props(v))
        }
        "UOL" => {
            r.u8()?;
            Ok(Node::Uol(str_block(r, d, img)?))
        }
        _ => Ok(Node::Other),
    }
}

/// 解出圖片像素（RGBA8，未預乘）
pub fn canvas_rgba(file: &WzFile, c: &CanvasRef) -> Result<Vec<u8>, String> {
    let d = file.data();
    // 遊戲檔損壞或更新到一半：位置、大小不合理就當作讀不到（不閃退）
    if c.data_off > d.len() {
        return Err("圖片資料超出檔案範圍".into());
    }
    let (w, h) = (c.w as usize, c.h as usize);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(format!("圖片大小不合理 {w}x{h}"));
    }
    let end = c.data_off.saturating_add(c.data_len).min(d.len());
    let raw = &d[c.data_off..end];
    let mut buf = Vec::new();
    let zlib_header = raw.len() >= 2 && raw[0] == 0x78 && matches!(raw[1], 0x01 | 0x5E | 0x9C | 0xDA);
    if zlib_header {
        let _ = flate2::read::ZlibDecoder::new(raw).read_to_end(&mut buf);
    } else {
        // 分段格式：[長度][資料]…（零金鑰，直接串接）
        let mut joined = Vec::new();
        let mut r = Reader::new(raw, 0);
        while r.p + 4 <= raw.len() {
            let n = r.i32()?;
            if n < 0 {
                break;
            }
            let s = r.p;
            let e = s.saturating_add(n as usize).min(raw.len());
            joined.extend_from_slice(&raw[s..e]);
            r.p = e;
        }
        let _ = flate2::read::ZlibDecoder::new(&joined[..]).read_to_end(&mut buf);
    }
    let mut px = vec![0u8; w * h * 4];
    match c.fmt {
        1 => {
            for i in 0..(w * h).min(buf.len() / 2) {
                let lo = buf[2 * i];
                let hi = buf[2 * i + 1];
                px[4 * i] = (hi & 0x0F) * 17;
                px[4 * i + 1] = (lo >> 4) * 17;
                px[4 * i + 2] = (lo & 0x0F) * 17;
                px[4 * i + 3] = (hi >> 4) * 17;
            }
        }
        2 => {
            for i in 0..(w * h).min(buf.len() / 4) {
                px[4 * i] = buf[4 * i + 2];
                px[4 * i + 1] = buf[4 * i + 1];
                px[4 * i + 2] = buf[4 * i];
                px[4 * i + 3] = buf[4 * i + 3];
            }
        }
        513 => {
            for i in 0..(w * h).min(buf.len() / 2) {
                let v = u16::from_le_bytes([buf[2 * i], buf[2 * i + 1]]) as u32;
                px[4 * i] = (((v >> 11) & 31) * 255 / 31) as u8;
                px[4 * i + 1] = (((v >> 5) & 63) * 255 / 63) as u8;
                px[4 * i + 2] = ((v & 31) * 255 / 31) as u8;
                px[4 * i + 3] = 255;
            }
        }
        f => return Err(format!("不支援的圖片格式 {f}")),
    }
    Ok(px)
}

/* ---------------- 分類（多個分割檔） ---------------- */

/// 一個分類資料夾，例如 `Data/Character/Hair`，包含 Hair.wz、Hair_000.wz…
pub struct Package {
    pub files: Vec<WzFile>,
    /// img 名稱 → (檔案索引, 位移)
    pub imgs: HashMap<String, (usize, usize)>,
    /// 子資料夾名稱（例如 _Canvas）
    pub subdirs: Vec<String>,
}

impl Package {
    pub fn open(dir: &Path, version_hint: Option<u32>) -> Result<Self, String> {
        let name = dir.file_name().and_then(|s| s.to_str()).ok_or("資料夾名稱錯誤")?.to_string();
        let mut paths: Vec<PathBuf> = Vec::new();
        let root = dir.join(format!("{name}.wz"));
        if root.exists() {
            paths.push(root);
        }
        // .ini 記錄最後一個分割檔編號；沒有 .ini 就逐一嘗試
        let last = std::fs::read_to_string(dir.join(format!("{name}.ini")))
            .ok()
            .and_then(|s| s.trim().rsplit('|').next().and_then(|n| n.trim().parse::<usize>().ok()));
        let max = last.unwrap_or(99);
        for i in 0..=max {
            let p = dir.join(format!("{name}_{i:03}.wz"));
            if p.exists() {
                paths.push(p);
            } else if last.is_none() {
                break;
            }
        }
        if paths.is_empty() {
            return Err(format!("找不到 {} 的 WZ 檔", dir.display()));
        }
        let mut files = Vec::new();
        let mut imgs = HashMap::new();
        let mut subdirs = Vec::new();
        let mut hint = version_hint;
        for p in paths {
            let f = WzFile::open(&p, hint)?;
            if f.version > 0 {
                hint = Some(f.version); // 同一分類的檔案版本相同，下一個檔案直接沿用
            }
            let idx = files.len();
            for e in f.root()? {
                if e.is_dir {
                    if !subdirs.contains(&e.name) {
                        subdirs.push(e.name);
                    }
                } else {
                    imgs.insert(e.name, (idx, e.offset));
                }
            }
            files.push(f);
        }
        Ok(Package { files, imgs, subdirs })
    }

    pub fn img(&self, name: &str) -> Result<Node, String> {
        let (fi, off) = *self.imgs.get(name).ok_or_else(|| format!("找不到 {name}"))?;
        parse_img(&self.files[fi], off)
    }

    pub fn file_of(&self, name: &str) -> Option<&WzFile> {
        self.imgs.get(name).map(|(fi, _)| &self.files[*fi])
    }
}

/// 整個遊戲的 Data 資料夾：依需要開啟各分類並快取
pub struct Data {
    pub root: PathBuf,
    packages: HashMap<String, Arc<Package>>,
    /// img 解析結果快取（分類路徑 + img 名稱），超過上限時先丟最舊的
    imgs: HashMap<String, Arc<Node>>,
    img_order: std::collections::VecDeque<String>,
    /// 已解壓縮的像素快取（避免每次重畫都重新解壓縮）
    pixels: HashMap<String, (u32, u32, Arc<Vec<u8>>)>,
    pixel_order: std::collections::VecDeque<String>,
    pixel_bytes: usize,
}

const IMG_CACHE_MAX: usize = 600;
const PIXEL_CACHE_BYTES: usize = 96 * 1024 * 1024;

impl Data {
    pub fn new(game_dir: &Path) -> Result<Self, String> {
        let root = if game_dir.join("Data").is_dir() { game_dir.join("Data") } else { game_dir.to_path_buf() };
        if !root.join("Character").is_dir() {
            return Err(format!("在 {} 找不到 Data\\Character 資料夾", game_dir.display()));
        }
        Ok(Data {
            root,
            packages: HashMap::new(),
            imgs: HashMap::new(),
            img_order: Default::default(),
            pixels: HashMap::new(),
            pixel_order: Default::default(),
            pixel_bytes: 0,
        })
    }

    /// 分類路徑，例如 "Character/Hair" 或 "Character/Hair/_Canvas"
    pub fn package(&mut self, rel: &str) -> Result<Arc<Package>, String> {
        if let Some(p) = self.packages.get(rel) {
            return Ok(p.clone());
        }
        let dir = rel.split('/').fold(self.root.clone(), |p, s| p.join(s));
        let p = Arc::new(Package::open(&dir, None)?);
        self.packages.insert(rel.to_string(), p.clone());
        Ok(p)
    }

    pub fn img(&mut self, rel: &str, name: &str) -> Result<Arc<Node>, String> {
        let key = format!("{rel}/{name}");
        if let Some(n) = self.imgs.get(&key) {
            return Ok(n.clone());
        }
        let pkg = self.package(rel)?;
        let n = Arc::new(pkg.img(name)?);
        while self.imgs.len() >= IMG_CACHE_MAX {
            match self.img_order.pop_front() {
                Some(old) => {
                    self.imgs.remove(&old);
                }
                None => break,
            }
        }
        self.img_order.push_back(key.clone());
        self.imgs.insert(key, n.clone());
        Ok(n)
    }

    fn cached_pixels(&self, key: &str) -> Option<(u32, u32, Vec<u8>)> {
        self.pixels.get(key).map(|(w, h, px)| (*w, *h, px.as_ref().clone()))
    }

    fn store_pixels(&mut self, key: String, w: u32, h: u32, px: &[u8]) {
        if px.len() > PIXEL_CACHE_BYTES / 8 {
            return;
        }
        while self.pixel_bytes + px.len() > PIXEL_CACHE_BYTES {
            match self.pixel_order.pop_front() {
                Some(old) => {
                    if let Some((_, _, v)) = self.pixels.remove(&old) {
                        self.pixel_bytes -= v.len();
                    }
                }
                None => break,
            }
        }
        self.pixel_bytes += px.len();
        self.pixel_order.push_back(key.clone());
        self.pixels.insert(key, (w, h, Arc::new(px.to_vec())));
    }

    /// 取得某個 img 內圖片節點的像素：優先使用 `_outlink`，其次 `_inlink`，最後是本身的資料
    pub fn canvas_pixels(&mut self, rel: &str, img: &str, inner: &str) -> Result<(u32, u32, Vec<u8>), String> {
        let node = self.img(rel, img)?;
        let target = node.path(inner).ok_or_else(|| format!("找不到 {rel}/{img}/{inner}"))?;
        if let Node::Canvas(c, props) = target {
            if let Some(link) = props.iter().find(|(k, _)| k == "_outlink").and_then(|(_, v)| v.as_str()) {
                return self.outlink_rgba(link);
            }
            if let Some(link) = props.iter().find(|(k, _)| k == "_inlink").and_then(|(_, v)| v.as_str()) {
                let link = link.to_string();
                return self.canvas_pixels(rel, img, &link);
            }
            let key = format!("{rel}/{img}@{}", c.data_off);
            if let Some(v) = self.cached_pixels(&key) {
                return Ok(v);
            }
            let pkg = self.package(rel)?;
            let file = pkg.file_of(img).ok_or("找不到圖片檔")?;
            let px = canvas_rgba(file, c)?;
            self.store_pixels(key, c.w, c.h, &px);
            return Ok((c.w, c.h, px));
        }
        Err(format!("{rel}/{img}/{inner} 不是圖片"))
    }

    /// 某個 img 最上層的屬性名稱（不解析內容、不快取）
    pub fn img_keys(&mut self, rel: &str, name: &str) -> Result<Vec<String>, String> {
        let pkg = self.package(rel)?;
        let (fi, off) = *pkg.imgs.get(name).ok_or_else(|| format!("找不到 {name}"))?;
        img_keys(&pkg.files[fi], off)
    }

    /// 只讀某個 img 最上層的一個屬性（不快取）
    pub fn img_child(&mut self, rel: &str, name: &str, key: &str) -> Result<Option<Node>, String> {
        let pkg = self.package(rel)?;
        let (fi, off) = *pkg.imgs.get(name).ok_or_else(|| format!("找不到 {name}"))?;
        parse_img_child(&pkg.files[fi], off, key)
    }

    /// 已取得的圖片節點（屬於 `rel/img`）轉成像素
    pub fn pixels_of(&mut self, rel: &str, img: &str, node: &Node) -> Result<(u32, u32, Vec<u8>), String> {
        let Node::Canvas(c, props) = node else { return Err("不是圖片".into()) };
        let prop = |k: &str| props.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.as_str()).map(str::to_string);
        if let Some(link) = prop("_outlink") {
            return self.outlink_rgba(&link);
        }
        if let Some(link) = prop("_inlink") {
            return self.canvas_pixels(rel, img, &link);
        }
        let key = format!("{rel}/{img}@{}", c.data_off);
        if let Some(v) = self.cached_pixels(&key) {
            return Ok(v);
        }
        let pkg = self.package(rel)?;
        let file = pkg.file_of(img).ok_or("找不到圖片檔")?;
        let px = canvas_rgba(file, c)?;
        self.store_pixels(key, c.w, c.h, &px);
        Ok((c.w, c.h, px))
    }

    /// 解析 `_outlink`，例如 "Character/Hair/_Canvas/00030000.img/default/hair"，回傳像素
    pub fn outlink_rgba(&mut self, link: &str) -> Result<(u32, u32, Vec<u8>), String> {
        if let Some(v) = self.cached_pixels(link) {
            return Ok(v);
        }
        let idx = link.find(".img/").ok_or("outlink 格式錯誤")? + 4;
        let (pkg_and_img, inner) = (&link[..idx], &link[idx + 1..]);
        let slash = pkg_and_img.rfind('/').ok_or("outlink 格式錯誤")?;
        let (rel, img) = (&pkg_and_img[..slash], &pkg_and_img[slash + 1..]);
        let node = self.img(rel, img)?;
        let pkg = self.package(rel)?;
        let file = pkg.file_of(img).ok_or("找不到圖片檔")?;
        match node.path(inner) {
            Some(Node::Canvas(c, _)) => {
                let px = canvas_rgba(file, c)?;
                self.store_pixels(link.to_string(), c.w, c.h, &px);
                Ok((c.w, c.h, px))
            }
            _ => Err(format!("找不到圖片 {link}")),
        }
    }
}
