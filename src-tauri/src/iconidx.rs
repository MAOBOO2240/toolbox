//! 點裝圖示索引：變更點裝能穿的所有裝備（帽子、臉飾、眼飾、耳環、上衣、套服、褲裙、鞋子、手套、披風、輔助武器、武器、名牌／聊天戒指）
//! 的小縮圖與顏色分類。給兩個功能用：
//! - 點裝辨識：丟一張遊戲截圖（裝備欄、背包、商城），把裡面每個道具格子拿去比對，找出是哪件點裝
//! - 變更點裝的顏色搜尋：「紅色」「白色帽子」「紅色刀」這種字
//!
//! 第一次使用要建立索引（讀幾萬個圖示，約一分鐘），之後存成檔案，遊戲版本沒變就直接讀檔。

use crate::avatar::{self, ItemOut};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// 縮圖邊長
const T: usize = 12;
const CACHE_MAGIC: &[u8; 4] = b"MBIX";
const CACHE_VER: u32 = 1;
/// 建索引時每批處理幾件（每批之間放開遊戲資料的鎖，不會卡住其他功能）
const BATCH: usize = 300;

/// 索引裡要收的欄位（跟變更點裝一樣；戒指只收名牌戒指、聊天戒指）
const SLOTS: [&str; 13] = ["Cap", "Face", "Eye", "Ear", "Coat", "Longcoat", "Pants", "Shoes", "Glove", "Cape", "Shield", "Weapon", "Ring"];
const SLOT_NAMES: [&str; 15] = ["Cap", "Face", "Eye", "Ear", "Coat", "Longcoat", "Pants", "Shoes", "Glove", "Cape", "Shield", "Weapon", "RingTag", "RingChat", "Ring"];

/// 顏色分類（位元）
const C_RED: u16 = 1;
const C_ORANGE: u16 = 2;
const C_YELLOW: u16 = 4;
const C_GREEN: u16 = 8;
const C_CYAN: u16 = 16;
const C_BLUE: u16 = 32;
const C_PURPLE: u16 = 64;
const C_PINK: u16 = 128;
const C_WHITE: u16 = 256;
const C_BLACK: u16 = 512;
const C_GRAY: u16 = 1024;
const C_BROWN: u16 = 2048;

#[derive(Clone)]
pub struct Entry {
    pub id: u32,
    pub slot: &'static str,
    pub name: String,
    pub cash: bool,
    /// T×T 的 RGB
    thumb: Vec<u8>,
    /// T×T，1 = 圖示有畫到的地方
    mask: Vec<u8>,
    /// 3×3 的平均顏色（粗略比對用）
    sig: [u8; 27],
    /// 顏色分類位元
    colors: u16,
}

#[derive(Default, Clone, Serialize)]
pub struct Status {
    pub ready: bool,
    pub building: bool,
    pub done: usize,
    pub total: usize,
    pub error: String,
    pub version: u32,
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static INDEX: Mutex<Option<Arc<Vec<Entry>>>> = Mutex::new(None);
static STATE: Mutex<Status> = Mutex::new(Status { ready: false, building: false, done: 0, total: 0, error: String::new(), version: 0 });

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn cache_path() -> PathBuf {
    DIR.get().cloned().unwrap_or_default().join("icon-index.bin")
}

fn state() -> std::sync::MutexGuard<'static, Status> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn index() -> Option<Arc<Vec<Entry>>> {
    INDEX.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn slot_static(s: &str) -> &'static str {
    SLOT_NAMES.iter().copied().find(|x| *x == s).unwrap_or("Ring")
}

/* ---------- 縮圖與顏色 ---------- */

/// 把一張 RGBA 圖裡「有畫到」的範圍縮成 T×T：回傳 (rgb, mask)。`paint` 判斷某個像素算不算有畫到
fn thumb_of(w: usize, h: usize, px: &[u8], paint: impl Fn(usize, usize) -> bool) -> Option<(Vec<u8>, Vec<u8>)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if paint(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 || x1 - x0 < 3 || y1 - y0 < 3 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let mut rgb = vec![0u8; T * T * 3];
    let mut mask = vec![0u8; T * T];
    for ty in 0..T {
        for tx in 0..T {
            // 這一格對應到原圖的範圍（面積平均）
            let sx0 = x0 + (tx as f32 * bw / T as f32).floor() as usize;
            let sx1 = (x0 + ((tx + 1) as f32 * bw / T as f32).ceil() as usize).min(x1).max(sx0 + 1);
            let sy0 = y0 + (ty as f32 * bh / T as f32).floor() as usize;
            let sy1 = (y0 + ((ty + 1) as f32 * bh / T as f32).ceil() as usize).min(y1).max(sy0 + 1);
            let (mut r, mut g, mut b, mut n, mut total) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    total += 1;
                    if paint(x, y) {
                        let i = (y * w + x) * 4;
                        r += px[i] as u32;
                        g += px[i + 1] as u32;
                        b += px[i + 2] as u32;
                        n += 1;
                    }
                }
            }
            let o = ty * T + tx;
            if n > 0 {
                rgb[o * 3] = (r / n) as u8;
                rgb[o * 3 + 1] = (g / n) as u8;
                rgb[o * 3 + 2] = (b / n) as u8;
            }
            mask[o] = (n * 2 >= total) as u8;
        }
    }
    Some((rgb, mask))
}

/// 3×3 的平均顏色（只算有畫到的格子）
fn sig_of(rgb: &[u8], mask: &[u8]) -> [u8; 27] {
    let mut out = [0u8; 27];
    for by in 0..3 {
        for bx in 0..3 {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for ty in by * 4..by * 4 + 4 {
                for tx in bx * 4..bx * 4 + 4 {
                    let o = ty * T + tx;
                    if mask[o] == 1 {
                        r += rgb[o * 3] as u32;
                        g += rgb[o * 3 + 1] as u32;
                        b += rgb[o * 3 + 2] as u32;
                        n += 1;
                    }
                }
            }
            let k = (by * 3 + bx) * 3;
            if n > 0 {
                out[k] = (r / n) as u8;
                out[k + 1] = (g / n) as u8;
                out[k + 2] = (b / n) as u8;
            }
        }
    }
    out
}

/// 一個像素屬於哪個顏色分類
fn color_class(r: u8, g: u8, b: u8) -> u16 {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max > 0.0 { (max - min) / max } else { 0.0 };
    if v < 0.2 {
        return C_BLACK;
    }
    if s < 0.16 {
        return if v > 0.82 { C_WHITE } else if v < 0.35 { C_BLACK } else { C_GRAY };
    }
    let d = max - min;
    let mut h = if max == r { (g - b) / d } else if max == g { 2.0 + (b - r) / d } else { 4.0 + (r - g) / d } * 60.0;
    if h < 0.0 {
        h += 360.0;
    }
    match h {
        x if x < 15.0 || x >= 340.0 => if v < 0.45 && s < 0.6 { C_BROWN } else if s < 0.45 && v > 0.75 { C_PINK } else { C_RED },
        x if x < 45.0 => if v < 0.62 { C_BROWN } else { C_ORANGE },
        x if x < 70.0 => C_YELLOW,
        x if x < 170.0 => C_GREEN,
        x if x < 200.0 => C_CYAN,
        x if x < 262.0 => C_BLUE,
        x if x < 300.0 => C_PURPLE,
        _ => C_PINK,
    }
}

/// 整張圖示的顏色分類：占比夠高的才算（黑色多半是外框，門檻高一點）
fn colors_of(w: usize, h: usize, px: &[u8]) -> u16 {
    let mut cnt: HashMap<u16, u32> = HashMap::new();
    let mut n = 0u32;
    for i in 0..w * h {
        if px[i * 4 + 3] > 128 {
            *cnt.entry(color_class(px[i * 4], px[i * 4 + 1], px[i * 4 + 2])).or_default() += 1;
            n += 1;
        }
    }
    if n == 0 {
        return 0;
    }
    let mut out = 0u16;
    let mut best = (0u16, 0u32);
    for (&c, &k) in &cnt {
        let share = k as f32 / n as f32;
        let th = match c {
            C_BLACK => 0.5,
            C_WHITE | C_GRAY => 0.3,
            _ => 0.18,
        };
        if share >= th {
            out |= c;
        }
        if k > best.1 {
            best = (c, k);
        }
    }
    out | best.0
}

fn make_entry(it: &ItemOut, w: u32, h: u32, px: &[u8]) -> Option<Entry> {
    let (w, h) = (w as usize, h as usize);
    if px.len() < w * h * 4 {
        return None;
    }
    let (thumb, mask) = thumb_of(w, h, px, |x, y| px[(y * w + x) * 4 + 3] > 40)?;
    let sig = sig_of(&thumb, &mask);
    Some(Entry { id: it.id, slot: slot_static(it.slot), name: it.name.clone(), cash: it.cash, thumb, mask, sig, colors: colors_of(w, h, px) })
}

/* ---------- 快取檔 ---------- */

fn save_cache(version: u32, list: &[Entry]) {
    let mut out = Vec::with_capacity(list.len() * (T * T * 4 + 64));
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VER.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(list.len() as u32).to_le_bytes());
    for e in list {
        out.extend_from_slice(&e.id.to_le_bytes());
        out.push(SLOT_NAMES.iter().position(|s| *s == e.slot).unwrap_or(14) as u8);
        out.push(e.cash as u8);
        let nb = e.name.as_bytes();
        out.extend_from_slice(&(nb.len().min(255) as u16).to_le_bytes());
        out.extend_from_slice(&nb[..nb.len().min(255)]);
        out.extend_from_slice(&e.colors.to_le_bytes());
        out.extend_from_slice(&e.sig);
        out.extend_from_slice(&e.mask);
        out.extend_from_slice(&e.thumb);
    }
    let _ = std::fs::create_dir_all(DIR.get().cloned().unwrap_or_default());
    let _ = std::fs::write(cache_path(), out);
}

fn load_cache(version: u32) -> Option<Vec<Entry>> {
    let b = std::fs::read(cache_path()).ok()?;
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> { let s = b.get(*p..*p + n)?; *p += n; Some(s) };
    if take(&mut p, 4)? != CACHE_MAGIC {
        return None;
    }
    let u32_at = |p: &mut usize| -> Option<u32> { Some(u32::from_le_bytes(take(p, 4)?.try_into().ok()?)) };
    if u32_at(&mut p)? != CACHE_VER || u32_at(&mut p)? != version {
        return None;
    }
    let n = u32_at(&mut p)? as usize;
    let mut list = Vec::with_capacity(n);
    for _ in 0..n {
        let id = u32_at(&mut p)?;
        let slot = SLOT_NAMES.get(*take(&mut p, 1)?.first()? as usize).copied()?;
        let cash = take(&mut p, 1)?[0] == 1;
        let nl = u16::from_le_bytes(take(&mut p, 2)?.try_into().ok()?) as usize;
        let name = String::from_utf8_lossy(take(&mut p, nl)?).into_owned();
        let colors = u16::from_le_bytes(take(&mut p, 2)?.try_into().ok()?);
        let sig: [u8; 27] = take(&mut p, 27)?.try_into().ok()?;
        let mask = take(&mut p, T * T)?.to_vec();
        let thumb = take(&mut p, T * T * 3)?.to_vec();
        list.push(Entry { id, slot, name, cash, thumb, mask, sig, colors });
    }
    Some(list)
}

/* ---------- 建立 ---------- */

/// 確認索引在不在（讀快取）；不在就回 false，要另外呼叫 build
fn ensure_loaded(hint: Option<&str>) -> Result<bool, String> {
    if index().is_some() {
        return Ok(true);
    }
    let version = avatar::with_ctx(hint, |ctx| Ok(ctx.game_version()))?;
    if let Some(list) = load_cache(version) {
        let n = list.len();
        *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(list));
        let mut s = state();
        *s = Status { ready: true, building: false, done: n, total: n, error: String::new(), version };
        return Ok(true);
    }
    Ok(false)
}

fn build(hint: Option<String>) {
    let h = hint.as_deref();
    let res: Result<(), String> = (|| {
        let (version, items) = avatar::with_ctx(h, |ctx| {
            let version = ctx.game_version();
            let mut all: Vec<ItemOut> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for s in SLOTS {
                if let Ok(v) = avatar::items_of(ctx, s) {
                    for it in v {
                        if seen.insert(it.id) {
                            all.push(it);
                        }
                    }
                }
            }
            Ok((version, all))
        })?;
        {
            let mut s = state();
            s.total = items.len();
            s.done = 0;
            s.version = version;
        }
        let mut list: Vec<Entry> = Vec::with_capacity(items.len());
        for chunk in items.chunks(BATCH) {
            avatar::with_ctx(h, |ctx| {
                for it in chunk {
                    if let Some((w, hh, px)) = ctx.icon_pixels(it.id) {
                        if let Some(e) = make_entry(it, w, hh, &px) {
                            list.push(e);
                        }
                    }
                }
                Ok(())
            })?;
            state().done += chunk.len();
        }
        save_cache(version, &list);
        *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(list));
        Ok(())
    })();
    let mut s = state();
    s.building = false;
    match res {
        Ok(()) => {
            s.ready = true;
            s.error.clear();
        }
        Err(e) => s.error = e,
    }
}

/// 索引狀態；沒建過就開始在背景建（要等 ready 才能比對）
#[tauri::command]
pub async fn iconidx_status(game_dir: Option<String>, start: Option<bool>) -> Status {
    let hint = game_dir.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        match ensure_loaded(hint.as_deref()) {
            Ok(true) => {}
            Ok(false) => {
                if start.unwrap_or(false) {
                    let mut s = state();
                    if !s.building {
                        s.building = true;
                        s.error.clear();
                        drop(s);
                        std::thread::spawn(move || build(hint));
                    }
                }
            }
            Err(e) => state().error = e,
        }
    })
    .await;
    state().clone()
}

/* ---------- 比對截圖 ---------- */

#[derive(Serialize, Clone)]
pub struct Hit {
    pub id: u32,
    pub name: String,
    pub slot: String,
    pub cash: bool,
    /// 差異分數，越小越像（0 = 一模一樣）
    pub score: f32,
}

#[derive(Serialize, Clone)]
pub struct Group {
    pub slot: String,
    pub items: Vec<Hit>,
}

#[derive(Serialize)]
pub struct MatchOut {
    pub groups: Vec<Group>,
    /// 圖裡找到幾個像道具格子的區塊
    pub candidates: usize,
    /// 圖裡有大塊的東西（例如整隻角色）：那部分的結果不可靠
    pub big: bool,
    /// 索引裡有幾件
    pub indexed: usize,
}

pub struct Img {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

pub fn decode_png(bytes: &[u8]) -> Result<Img, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| format!("圖片讀不出來：{e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("圖片讀不出來：{e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let px: Vec<u8> = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => bytes.to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => bytes.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => bytes.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        _ => return Err("請用一般的 PNG（8 位元）".into()),
    };
    if w == 0 || h == 0 || w * h > 16_000_000 {
        return Err("圖片太大或是空的".into());
    }
    Ok(Img { w, h, px })
}

/// 找出圖裡「像道具格子」的區塊：先找出背景色（大面積的平色），其他像素連在一起的就是一個區塊
pub fn find_boxes(img: &Img) -> (Vec<(usize, usize, usize, usize)>, Vec<(usize, usize, usize, usize)>, Vec<bool>) {
    let (w, h) = (img.w, img.h);
    let n = w * h;
    // 顏色量化成 5 bit，統計；占比 ≥ 1.5% 的當背景色
    let mut hist: HashMap<u16, u32> = HashMap::new();
    let q = |i: usize| -> u16 {
        let p = &img.px[i * 4..i * 4 + 4];
        if p[3] < 128 {
            return 0xFFFF;
        }
        ((p[0] as u16 >> 3) << 10) | ((p[1] as u16 >> 3) << 5) | (p[2] as u16 >> 3)
    };
    for i in 0..n {
        *hist.entry(q(i)).or_default() += 1;
    }
    let mut bg: Vec<[i32; 3]> = hist
        .iter()
        .filter(|(_, &c)| c as f32 / n as f32 >= 0.015)
        .filter_map(|(&k, _)| if k == 0xFFFF { None } else { Some([((k >> 10) & 31) as i32 * 8 + 4, ((k >> 5) & 31) as i32 * 8 + 4, (k & 31) as i32 * 8 + 4]) })
        .collect();
    if bg.is_empty() {
        bg.push([255, 255, 255]);
    }
    let is_bg = |i: usize| -> bool {
        let p = &img.px[i * 4..i * 4 + 4];
        if p[3] < 128 {
            return true;
        }
        bg.iter().any(|c| (c[0] - p[0] as i32).abs() <= 14 && (c[1] - p[1] as i32).abs() <= 14 && (c[2] - p[2] as i32).abs() <= 14)
    };
    let mut paint: Vec<bool> = (0..n).map(|i| !is_bg(i)).collect();
    // 稍微膨脹一格，讓同一個圖示分開的小碎片連在一起
    let orig = paint.clone();
    for y in 0..h {
        for x in 0..w {
            if orig[y * w + x] {
                continue;
            }
            let near = (x > 0 && orig[y * w + x - 1]) || (x + 1 < w && orig[y * w + x + 1]) || (y > 0 && orig[(y - 1) * w + x]) || (y + 1 < h && orig[(y + 1) * w + x]);
            if near {
                paint[y * w + x] = true;
            }
        }
    }
    // 連通區塊（4 連通，用 stack）
    let mut seen = vec![false; n];
    let mut small = Vec::new();
    let mut big = Vec::new();
    let mut stack = Vec::new();
    for start in 0..n {
        if !paint[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut cnt) = (w, h, 0usize, 0usize, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            cnt += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            let mut push = |j: usize| {
                if paint[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        if bw < 10 || bh < 10 || cnt < 60 {
            continue;
        }
        if bw.max(bh) <= 96 {
            small.push((x0, y0, x1, y1));
        } else {
            big.push((x0, y0, x1, y1));
        }
    }
    (small, big, orig)
}

/// 一個區塊的縮圖（只算非背景的像素）
fn crop_thumb(img: &Img, paint: &[bool], (x0, y0, x1, y1): (usize, usize, usize, usize)) -> Option<(Vec<u8>, Vec<u8>)> {
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut px = vec![0u8; cw * ch * 4];
    for y in 0..ch {
        for x in 0..cw {
            let si = ((y0 + y) * img.w + x0 + x) * 4;
            let di = (y * cw + x) * 4;
            px[di..di + 3].copy_from_slice(&img.px[si..si + 3]);
            px[di + 3] = if paint[(y0 + y) * img.w + x0 + x] { 255 } else { 0 };
        }
    }
    thumb_of(cw, ch, &px, |x, y| px[(y * cw + x) * 4 + 3] > 0)
}

/// 兩張縮圖的差異：只看索引圖示有畫到的格子，加上「應該有畫卻是背景」的懲罰
fn score(e: &Entry, rgb: &[u8], mask: &[u8]) -> f32 {
    let (mut diff, mut n, mut missing, mut extra) = (0f32, 0f32, 0f32, 0f32);
    for o in 0..T * T {
        let (tx, ty) = (o % T, o / T);
        // 遊戲會在圖示的左下（鎖頭）、右下（點裝的 C）疊小標誌，那兩塊的差異算輕一點
        let w = if ty >= 8 && (tx >= 8 || tx < 4) { 0.3 } else { 1.0 };
        match (e.mask[o], mask[o]) {
            (1, 1) => {
                n += w;
                diff += w * ((e.thumb[o * 3] as i32 - rgb[o * 3] as i32).unsigned_abs() + (e.thumb[o * 3 + 1] as i32 - rgb[o * 3 + 1] as i32).unsigned_abs() + (e.thumb[o * 3 + 2] as i32 - rgb[o * 3 + 2] as i32).unsigned_abs()) as f32;
            }
            (1, 0) => missing += w,
            (0, 1) => extra += w,
            _ => {}
        }
    }
    if n < 8.0 {
        return 999.0;
    }
    let avg = diff / n / 3.0; // 0..255
    avg + missing * 3.5 + extra * 1.5
}

fn sig_dist(a: &[u8; 27], b: &[u8; 27]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).sum()
}

/// 拿一張縮圖去索引裡找最像的幾件
fn best_matches(idx: &[Entry], rgb: &[u8], mask: &[u8], cash_only: bool, k: usize) -> Vec<(usize, f32)> {
    let sig = sig_of(rgb, mask);
    // 先用 3×3 粗比對挑 500 件，再精算
    let mut pre: Vec<(usize, u32)> = idx.iter().enumerate().filter(|(_, e)| !cash_only || e.cash).map(|(i, e)| (i, sig_dist(&e.sig, &sig))).collect();
    pre.sort_unstable_by_key(|x| x.1);
    pre.truncate(500);
    let mut out: Vec<(usize, f32)> = pre.into_iter().map(|(i, _)| (i, score(&idx[i], rgb, mask))).collect();
    out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(k);
    out
}

/// 比對一張截圖（PNG 的 base64）：回傳每個欄位最像的幾件
#[tauri::command]
pub async fn icon_match(game_dir: Option<String>, png_b64: String, cash_only: Option<bool>) -> Result<MatchOut, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !ensure_loaded(game_dir.as_deref())? {
            return Err("圖示索引還沒建立".into());
        }
        let idx = index().ok_or("圖示索引還沒建立")?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(png_b64.trim()).map_err(|_| "圖片資料不對")?;
        let img = decode_png(&bytes)?;
        let cash_only = cash_only.unwrap_or(true);
        let (small, big, paint) = find_boxes(&img);
        // 每個欄位：id → 最佳分數
        let mut best: HashMap<&'static str, HashMap<u32, f32>> = HashMap::new();
        let mut add = |i: usize, s: f32| {
            let e = &idx[i];
            let m = best.entry(e.slot).or_default();
            let v = m.entry(e.id).or_insert(s);
            if s < *v {
                *v = s;
            }
        };
        for b in &small {
            if let Some((rgb, mask)) = crop_thumb(&img, &paint, *b) {
                for (i, s) in best_matches(&idx, &rgb, &mask, cash_only, 6) {
                    if s < 70.0 {
                        add(i, s);
                    }
                }
            }
        }
        // 大塊的（整隻角色之類）：在裡面滑動 32 格的窗試試看，門檻嚴一點，結果只是參考
        let big_any = !big.is_empty();
        let mut windows = 0;
        'outer: for &(x0, y0, x1, y1) in &big {
            let mut y = y0;
            while y + 24 <= y1 {
                let mut x = x0;
                while x + 24 <= x1 {
                    let bx = (x, y, (x + 36).min(x1), (y + 36).min(y1));
                    if let Some((rgb, mask)) = crop_thumb(&img, &paint, bx) {
                        for (i, s) in best_matches(&idx, &rgb, &mask, cash_only, 3) {
                            if s < 38.0 {
                                add(i, s + 20.0);
                            }
                        }
                    }
                    windows += 1;
                    if windows > 600 {
                        break 'outer;
                    }
                    x += 12;
                }
                y += 12;
            }
        }
        let order = ["Cap", "Face", "Eye", "Ear", "Longcoat", "Coat", "Pants", "Shoes", "Glove", "Cape", "Weapon", "Shield", "RingTag", "RingChat"];
        let mut groups = Vec::new();
        for slot in order {
            let Some(m) = best.get(slot) else { continue };
            let mut items: Vec<Hit> = m
                .iter()
                .map(|(&id, &s)| {
                    let e = idx.iter().find(|e| e.id == id).unwrap();
                    Hit { id, name: e.name.clone(), slot: slot.to_string(), cash: e.cash, score: (s * 10.0).round() / 10.0 }
                })
                .collect();
            items.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
            items.truncate(12);
            groups.push(Group { slot: slot.to_string(), items });
        }
        Ok(MatchOut { groups, candidates: small.len(), big: big_any, indexed: idx.len() })
    })
    .await
    .map_err(|e| e.to_string())?
}

/* ---------- 顏色搜尋 ---------- */

/// 把「紅色刀」「白色帽子」拆成（顏色位元, 其餘文字）；不是顏色開頭就回 None
pub fn parse_color_query(q: &str) -> Option<(u16, String)> {
    const WORDS: [(&str, u16); 20] = [
        ("粉紅", C_PINK), ("粉", C_PINK), ("桃", C_PINK), ("紅", C_RED), ("橘", C_ORANGE), ("橙", C_ORANGE), ("黃", C_YELLOW), ("金", C_YELLOW),
        ("綠", C_GREEN), ("青", C_CYAN), ("藍", C_BLUE), ("紫", C_PURPLE), ("白", C_WHITE), ("黑", C_BLACK), ("灰", C_GRAY), ("銀", C_GRAY),
        ("棕", C_BROWN), ("咖啡", C_BROWN), ("褐", C_BROWN), ("深", 0),
    ];
    let q = q.trim();
    let mut rest = q;
    let mut bits = 0u16;
    loop {
        let mut hit = false;
        for (w, b) in WORDS {
            if let Some(r) = rest.strip_prefix(w) {
                bits |= b;
                rest = r.strip_prefix("色").unwrap_or(r);
                hit = true;
                break;
            }
        }
        if !hit {
            break;
        }
    }
    if bits == 0 {
        return None;
    }
    Some((bits, rest.trim().trim_start_matches(['的', ' ']).to_string()))
}

/// 其餘文字對到的欄位（帽子→Cap…），沒有就 None
fn slot_of_word(w: &str) -> Option<&'static [&'static str]> {
    Some(match w {
        "帽子" | "帽" => &["Cap"],
        "臉飾" => &["Face"],
        "眼飾" | "眼鏡" => &["Eye"],
        "耳環" | "耳飾" => &["Ear"],
        "上衣" | "衣服" | "衣" => &["Coat", "Longcoat"],
        "套服" | "套裝" | "洋裝" | "連身" => &["Longcoat"],
        "褲子" | "褲裙" | "褲" | "裙子" | "裙" => &["Pants"],
        "鞋子" | "鞋" => &["Shoes"],
        "手套" => &["Glove"],
        "披風" => &["Cape"],
        "武器" => &["Weapon"],
        "輔助武器" | "副手" | "盾" => &["Shield"],
        "戒指" => &["RingTag", "RingChat"],
        _ => return None,
    })
}

/// 顏色搜尋：回傳符合顏色（與其餘文字）的道具
#[tauri::command]
pub async fn iconidx_color_search(game_dir: Option<String>, query: String) -> Result<Vec<ItemOut>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some((bits, rest)) = parse_color_query(&query) else { return Ok(Vec::new()) };
        if !ensure_loaded(game_dir.as_deref())? {
            return Err("圖示索引還沒建立".into());
        }
        let idx = index().ok_or("圖示索引還沒建立")?;
        let slots = slot_of_word(&rest);
        let out: Vec<ItemOut> = idx
            .iter()
            .filter(|e| e.colors & bits == bits)
            .filter(|e| rest.is_empty() || slots.map(|s| s.contains(&e.slot)).unwrap_or(false) || e.name.contains(&rest))
            .map(|e| ItemOut { id: e.id, name: e.name.clone(), slot: e.slot, cash: e.cash, dye: None })
            .collect();
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_words() {
        assert_eq!(parse_color_query("紅色刀"), Some((C_RED, "刀".into())));
        assert_eq!(parse_color_query("白色帽子"), Some((C_WHITE, "帽子".into())));
        assert_eq!(parse_color_query("粉紅洋裝"), Some((C_PINK, "洋裝".into())));
        assert_eq!(parse_color_query("藍"), Some((C_BLUE, String::new())));
        assert_eq!(parse_color_query("帽子"), None);
    }

    #[test]
    fn classes() {
        assert_eq!(color_class(220, 30, 30), C_RED);
        assert_eq!(color_class(250, 250, 250), C_WHITE);
        assert_eq!(color_class(20, 20, 20), C_BLACK);
        assert_eq!(color_class(40, 90, 220), C_BLUE);
        assert_eq!(color_class(250, 170, 200), C_PINK);
    }

    #[test]
    fn thumb_and_score() {
        // 一張 20×20 的紅色方塊，縮圖後跟自己比對分數應該是 0
        let (w, h) = (20usize, 20usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 4..16 {
            for x in 4..16 {
                let i = (y * w + x) * 4;
                px[i] = 200;
                px[i + 1] = 30;
                px[i + 2] = 30;
                px[i + 3] = 255;
            }
        }
        let it = ItemOut { id: 1, name: "x".into(), slot: "Cap", cash: true, dye: None };
        let e = make_entry(&it, w as u32, h as u32, &px).unwrap();
        assert_eq!(score(&e, &e.thumb, &e.mask), 0.0);
        assert!(e.colors & C_RED != 0);
    }
}
