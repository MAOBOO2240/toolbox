//! 紙娃娃：從遊戲 WZ 檔組合身體、頭、臉型與髮型，輸出 PNG。

use crate::wz::{Data, Node};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const HAIR_COLORS: [&str; 8] = ["黑", "紅", "橘", "黃", "綠", "藍", "紫", "褐"];

struct Ctx {
    dir: PathBuf,
    data: Data,
    zmap: HashMap<String, i32>,
    hair: Vec<(u32, String)>,
    face: Vec<(u32, String)>,
    skin: Vec<(u32, String)>,
    /// 裝備（點裝）：編號、名稱、分類資料夾
    equip: Vec<(u32, String, &'static str)>,
    /// 圖層 z 名稱 → 佔用的欄位代碼（Base/smap.img）
    smap: HashMap<String, String>,
    /// 道具圖示快取
    icons: HashMap<u32, Option<String>>,
    /// 各欄位的道具清單快取
    lists: HashMap<String, Vec<ItemOut>>,
}

/// 點裝欄位：飾品依編號再細分成臉飾、眼飾、耳環
fn slot_key(id: u32, cat: &str) -> &'static str {
    match cat {
        "Accessory" => match id / 10000 {
            101 => "Face",
            102 => "Eye",
            103 => "Ear",
            _ => "Acc",
        },
        "Cap" => "Cap",
        "Coat" => "Coat",
        "Longcoat" => "Longcoat",
        "Pants" => "Pants",
        "Shoes" => "Shoes",
        "Glove" => "Glove",
        "Cape" => "Cape",
        "Shield" => "Shield",
        "Weapon" => "Weapon",
        _ => "Acc",
    }
}

fn slot_cat(slot: &str) -> Option<(&'static str, Option<u32>)> {
    Some(match slot {
        "Face" => ("Accessory", Some(101)),
        "Eye" => ("Accessory", Some(102)),
        "Ear" => ("Accessory", Some(103)),
        "Cap" => ("Cap", None),
        "Coat" => ("Coat", None),
        "Longcoat" => ("Longcoat", None),
        "Pants" => ("Pants", None),
        "Shoes" => ("Shoes", None),
        "Glove" => ("Glove", None),
        "Cape" => ("Cape", None),
        "Shield" => ("Shield", None),
        "Weapon" => ("Weapon", None),
        _ => return None,
    })
}

#[derive(Serialize, Clone)]
pub struct ItemOut {
    pub id: u32,
    pub name: String,
    pub slot: &'static str,
    /// 是否為點裝（info/cash = 1）
    pub cash: bool,
    /// 染色（只有紙娃娃輸出的裝備清單會帶）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dye: Option<Dye>,
}

/// 時裝染色（與遊戲的染色稜鏡相同的參數）
/// range：0 全色系、1 紅、2 黃、3 綠、4 青、5 藍、6 紫
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Dye {
    #[serde(default)]
    pub range: u8,
    #[serde(default)]
    pub hue: i32,
    #[serde(default)]
    pub sat: i32,
    #[serde(default)]
    pub val: i32,
}

impl Dye {
    fn is_noop(&self) -> bool {
        self.hue.rem_euclid(360) == 0 && self.sat == 0 && self.val == 0
    }
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r + m, g + m, b + m)
}

/// 依色系範圍調整色相、飽和度、亮度
fn apply_dye(px: &mut [u8], dye: &Dye) {
    if dye.is_noop() {
        return;
    }
    let (lo, hi) = match dye.range {
        1 => (330.0, 30.0),
        2 => (30.0, 90.0),
        3 => (90.0, 150.0),
        4 => (150.0, 210.0),
        5 => (210.0, 270.0),
        6 => (270.0, 330.0),
        _ => (0.0, 360.0),
    };
    let in_range = |h: f32| if dye.range == 0 { true } else if lo > hi { h >= lo || h < hi } else { h >= lo && h < hi };
    let ds = dye.sat.clamp(-100, 100) as f32 / 100.0;
    let dv = dye.val.clamp(-100, 100) as f32 / 100.0;
    for p in px.chunks_exact_mut(4) {
        if p[3] == 0 {
            continue;
        }
        let (h, s, v) = rgb_to_hsv(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        // 灰階（黑線、白色）沒有色相，只在全色系時調整亮度
        if s < 0.04 && dye.range != 0 {
            continue;
        }
        if !in_range(h) {
            continue;
        }
        let h2 = h + dye.hue as f32;
        let s2 = if ds >= 0.0 { s + (1.0 - s) * ds } else { s * (1.0 + ds) };
        let v2 = if dv >= 0.0 { v + (1.0 - v) * dv } else { v * (1.0 + dv) };
        let (r, g, b) = hsv_to_rgb(h2, s2.clamp(0.0, 1.0), v2.clamp(0.0, 1.0));
        p[0] = (r * 255.0).round() as u8;
        p[1] = (g * 255.0).round() as u8;
        p[2] = (b * 255.0).round() as u8;
    }
}

/// 紙娃娃要穿的一件點裝：用編號或名稱指定，可以帶染色
#[derive(Deserialize, Clone, Default)]
pub struct EquipReq {
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub dye: Option<Dye>,
}

const EQUIP_CATS: [&str; 10] = ["Cap", "Accessory", "Coat", "Longcoat", "Pants", "Shoes", "Glove", "Cape", "Shield", "Weapon"];

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

/* ---------------- 找遊戲資料夾 ---------------- */

fn valid_dir(p: &Path) -> bool {
    p.join("Data").join("Character").join("Hair").is_dir() || p.join("Character").join("Hair").is_dir()
}

fn find_game_dir(hint: Option<&str>) -> Option<PathBuf> {
    if let Some(h) = hint.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(h);
        // 使用者可能選到 Data 資料夾或其中的子資料夾，往上找
        for anc in p.ancestors().take(4) {
            if valid_dir(anc) {
                return Some(anc.to_path_buf());
            }
        }
    }
    for d in ["C", "D", "E", "F", "G", "H"] {
        for sub in [
            r"Program Files\gamania Games\MapleStory",
            r"Program Files (x86)\gamania Games\MapleStory",
            r"gamania Games\MapleStory",
            r"Gamania\MapleStory",
            r"MapleStory",
        ] {
            let p = PathBuf::from(format!(r"{d}:\{sub}"));
            if valid_dir(&p) {
                return Some(p);
            }
        }
    }
    None
}

fn names_of(eqp: &Node, cat: &str) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = eqp
        .path(&format!("Eqp/{cat}"))
        .map(|n| {
            n.children()
                .iter()
                .filter_map(|(id, item)| Some((id.parse().ok()?, item.get("name")?.as_str()?.trim().to_string())))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn open_ctx(dir: PathBuf) -> Result<Ctx, String> {
    let mut data = Data::new(&dir)?;
    let zmap = data
        .img("Base", "zmap.img")
        .map(|z| z.children().iter().enumerate().map(|(i, (k, _))| (k.clone(), i as i32)).collect())
        .unwrap_or_default();
    let smap = data
        .img("Base", "smap.img")
        .map(|z| z.children().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let eqp = data.img("String", "Eqp.img")?;
    let mut equip = Vec::new();
    for cat in EQUIP_CATS {
        for (id, name) in names_of(&eqp, cat) {
            equip.push((id, name, cat));
        }
    }
    let ctx = Ctx {
        equip,
        smap,
        icons: HashMap::new(),
        lists: HashMap::new(),
        hair: names_of(&eqp, "Hair"),
        face: names_of(&eqp, "Face"),
        skin: names_of(&eqp, "Skin"),
        dir,
        data,
        zmap,
    };
    Ok(ctx)
}

/* ---------------- 名稱 → 編號 ---------------- */

fn hair_color_idx(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains('棕') {
        return Some(7);
    }
    HAIR_COLORS.iter().position(|c| s.contains(c)).map(|i| i as u32)
}

fn face_color_idx(s: &str) -> Option<u32> {
    let s = s.trim();
    let table: [(&str, u32); 14] = [
        ("祖母綠", 5),
        ("藍寶石", 5),
        ("紫水晶", 7),
        ("水晶", 8),
        ("白", 8),
        ("黑", 0),
        ("藍", 1),
        ("紅", 2),
        ("綠", 3),
        ("褐", 4),
        ("棕", 4),
        ("紫", 6),
        ("粉", 7),
        ("桃", 7),
    ];
    table.iter().find(|(k, _)| s.contains(k)).map(|(_, v)| *v)
}

fn strip_hair_color(name: &str) -> &str {
    for c in HAIR_COLORS {
        if let Some(rest) = name.strip_prefix(c) {
            return rest.strip_prefix('色').unwrap_or(rest);
        }
    }
    name
}

/// 依名稱找出同一系列（同造型不同顏色）的編號。同名有多個系列時，男用較小的、女用較大的。
fn pick_family(list: &[(u32, String)], name: &str, family: impl Fn(u32) -> u32, female: bool, hair: bool) -> Option<(u32, u32)> {
    let name = name.trim();
    let mut hits: Vec<u32> = list.iter().filter(|(_, n)| n == name).map(|(id, _)| *id).collect();
    if hits.is_empty() && hair {
        // 名稱的顏色和遊戲裡不同時（例如「黑色○○」找不到），改用去掉顏色後比對
        let base = strip_hair_color(name);
        hits = list.iter().filter(|(_, n)| strip_hair_color(n) == base).map(|(id, _)| *id).collect();
    }
    if hits.is_empty() {
        return None;
    }
    let mut fams: Vec<u32> = hits.iter().map(|&id| family(id)).collect();
    fams.sort();
    fams.dedup();
    let fam = if female { *fams.last()? } else { fams[0] };
    let id = hits.into_iter().find(|&id| family(id) == fam)?;
    Some((fam, id))
}

/* ---------------- 圖層 ---------------- */

#[derive(Clone)]
struct Layer {
    name: String,
    /// z 圖層名稱（例如 hairOverHead）
    zname: String,
    /// 0 = 身體／頭，1 = 臉型，2 = 髮型，10 以上 = 裝備
    owner: usize,
    z: i32,
    origin: (i32, i32),
    map: Vec<(String, (i32, i32))>,
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// 依路徑取得節點，途中遇到 UOL 會跟著跳轉
fn resolve<'a>(root: &'a Node, path: &str, depth: u32) -> Option<&'a Node> {
    if depth > 8 {
        return None;
    }
    let mut segs: Vec<&str> = Vec::new();
    for s in path.split('/').filter(|s| !s.is_empty()) {
        if s == ".." {
            segs.pop();
        } else {
            segs.push(s);
        }
    }
    let mut cur = root;
    for (i, s) in segs.iter().enumerate() {
        cur = cur.get(s)?;
        if let Node::Uol(target) = cur {
            let mut p = segs[..i].join("/");
            p.push('/');
            p.push_str(target);
            for rest in &segs[i + 1..] {
                p.push('/');
                p.push_str(rest);
            }
            return resolve(root, &p, depth + 1);
        }
    }
    Some(cur)
}

impl Ctx {
    fn z_of(&self, z: &str) -> i32 {
        self.zmap.get(z).copied().or_else(|| self.zmap.get("head").copied()).unwrap_or(0)
    }

    fn has(&mut self, rel: &str, id: u32) -> bool {
        self.data.package(rel).map(|p| p.imgs.contains_key(&format!("{id:08}.img"))).unwrap_or(false)
    }

    /// 讀取某個 img 的某個畫格中的所有圖片（只取 `only` 列出的名稱，空的話全部）
    fn layers(&mut self, rel: &str, id: u32, frame: &str, only: &[&str], skin: u32) -> Result<Vec<Layer>, String> {
        let img = format!("{id:08}.img");
        let root = self.data.img(rel, &img)?;
        let fnode = resolve(&root, frame, 0).ok_or_else(|| format!("{rel}/{img} 沒有 {frame}"))?;
        let keys: Vec<String> = fnode.children().iter().map(|(k, _)| k.clone()).collect();
        let mut out = Vec::new();
        for k in keys {
            if !only.is_empty() && !only.contains(&k.as_str()) {
                continue;
            }
            let Some(mut node) = resolve(&root, &format!("{frame}/{k}"), 0) else { continue };
            if let Node::Props(_) = node {
                // hairShade 依膚色分組：0、1、2…
                let sub = if node.get(&skin.to_string()).is_some() { skin.to_string() } else { "0".to_string() };
                match resolve(&root, &format!("{frame}/{k}/{sub}"), 0) {
                    Some(n) => node = n,
                    None => continue,
                }
            }
            let Node::Canvas(_, props) = node else { continue };
            let get = |n: &str| props.iter().find(|(p, _)| p == n).map(|(_, v)| v);
            let origin = get("origin").and_then(|v| v.as_vec()).unwrap_or((0, 0));
            let z = get("z").and_then(|v| v.as_str()).unwrap_or(k.as_str()).to_string();
            let map = get("map")
                .map(|m| m.children().iter().filter_map(|(mk, mv)| Some((mk.clone(), mv.as_vec()?))).collect())
                .unwrap_or_default();
            let node = node.clone();
            let Ok((w, h, px)) = self.data.pixels_of(rel, &img, &node) else { continue };
            if w <= 1 && h <= 1 {
                continue;
            }
            out.push(Layer { name: k, z: self.z_of(&z), zname: z, owner: 0, origin, map, w, h, px });
        }
        Ok(out)
    }

    /// 依名稱找點裝：只取遊戲資料裡真的有圖的，同名時優先選符合性別的
    fn find_equip(&mut self, name: &str, female: bool) -> Option<(u32, &'static str)> {
        let name = name.trim();
        let mut hits: Vec<(u32, &'static str)> = self.equip.iter().filter(|(_, n, _)| n == name).map(|(id, _, c)| (*id, *c)).collect();
        hits.retain(|(id, cat)| {
            let rel = format!("Character/{cat}");
            self.has(&rel, *id)
        });
        let gender_ok = |id: u32| match id / 1000 % 10 {
            0 => !female,
            1 => female,
            _ => true,
        };
        hits.iter().copied().find(|(id, _)| gender_ok(*id)).or_else(|| hits.first().copied())
    }

    fn equip_by_id(&mut self, id: u32) -> Option<(u32, &'static str)> {
        let cat = self.equip.iter().find(|(i, _, _)| *i == id).map(|(_, _, c)| *c)?;
        self.has(&format!("Character/{cat}"), id).then_some((id, cat))
    }

    fn equip_name(&self, id: u32) -> String {
        self.equip.iter().find(|(i, _, _)| *i == id).map(|(_, n, _)| n.clone()).unwrap_or_default()
    }

    fn icon(&mut self, id: u32) -> Option<String> {
        if let Some(v) = self.icons.get(&id) {
            return v.clone();
        }
        let v = (|| {
            let (_, cat) = self.equip_by_id(id)?;
            let rel = format!("Character/{cat}");
            let img = format!("{id:08}.img");
            // 只讀 info，不解析整個道具（快很多，也不會把紙娃娃用的快取擠掉）
            let info = self.data.img_child(&rel, &img, "info").ok()??;
            let node = ["icon", "iconRaw"].iter().find_map(|k| info.get(k).filter(|n| matches!(n, Node::Canvas(..))))?.clone();
            let (w, h, px) = self.data.pixels_of(&rel, &img, &node).ok()?;
            if w <= 1 && h <= 1 {
                return None;
            }
            png_data_url(w, h, &px).ok()
        })();
        self.icons.insert(id, v.clone());
        v
    }

    /// 裝備的畫格：一般是 stand1/0；點裝武器會依武器種類再分一層（例如 30/stand1/0）；帽子、飾品用 default
    fn equip_frame(&mut self, rel: &str, id: u32, pose: &str) -> Option<String> {
        let root = self.data.img(rel, &format!("{id:08}.img")).ok()?;
        let has = |p: &str| resolve(&root, p, 0).is_some();
        for cand in [format!("{pose}/0"), "stand1/0".to_string(), "stand2/0".to_string(), "default".to_string()] {
            if has(&cand) {
                return Some(cand);
            }
        }
        for (k, _) in root.children() {
            if k.chars().all(|c| c.is_ascii_digit()) {
                for cand in [format!("{k}/{pose}/0"), format!("{k}/stand1/0"), format!("{k}/stand2/0")] {
                    if has(&cand) {
                        return Some(cand);
                    }
                }
            }
        }
        None
    }

    fn vslot(&mut self, rel: &str, id: u32) -> String {
        self.data
            .img(rel, &format!("{id:08}.img"))
            .ok()
            .and_then(|r| r.path("info/vslot").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_default()
    }

    /// 混染：兩種顏色的同名圖層依比例混合
    fn mixed(&mut self, rel: &str, base: u32, mix: Option<(u32, f32)>, frame: &str, skin: u32) -> Result<Vec<Layer>, String> {
        let mut a = self.layers(rel, base, frame, &[], skin)?;
        if let Some((mid, rate)) = mix {
            let rate = (rate / 100.0).clamp(0.0, 1.0);
            if rate > 0.0 && mid != base && self.has(rel, mid) {
                if let Ok(b) = self.layers(rel, mid, frame, &[], skin) {
                    for la in a.iter_mut() {
                        if let Some(lb) = b.iter().find(|l| l.name == la.name && l.w == la.w && l.h == la.h) {
                            for (pa, pb) in la.px.iter_mut().zip(lb.px.iter()) {
                                *pa = (*pa as f32 * (1.0 - rate) + *pb as f32 * rate).round() as u8;
                            }
                        }
                    }
                }
            }
        }
        Ok(a)
    }
}

/// 依錨點（navel、neck、brow…）擺放並依 zmap 疊圖，回傳裁切後的 RGBA
fn compose(parts: &[Layer], start: &[(&str, (i32, i32))]) -> Option<(u32, u32, Vec<u8>)> {
    let mut anchors: HashMap<String, (i32, i32)> = start.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let mut placed: Vec<(i32, usize, i32, i32, &Layer)> = Vec::new();
    for (i, l) in parts.iter().enumerate() {
        let pos = l
            .map
            .iter()
            .find_map(|(k, (mx, my))| anchors.get(k).map(|(ax, ay)| (ax - mx, ay - my)))
            .unwrap_or((0, 0));
        for (k, (mx, my)) in &l.map {
            anchors.entry(k.clone()).or_insert((pos.0 + mx, pos.1 + my));
        }
        placed.push((l.z, i, pos.0 - l.origin.0, pos.1 - l.origin.1, l));
    }
    if placed.is_empty() {
        return None;
    }
    // zmap 越前面越上層，所以從索引大的開始畫；同一層依加入順序
    placed.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let x0 = placed.iter().map(|p| p.2).min()?;
    let y0 = placed.iter().map(|p| p.3).min()?;
    let x1 = placed.iter().map(|p| p.2 + p.4.w as i32).max()?;
    let y1 = placed.iter().map(|p| p.3 + p.4.h as i32).max()?;
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if w == 0 || h == 0 || w > 4000 || h > 4000 {
        return None;
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (_, _, x, y, l) in placed {
        let (ox, oy) = ((x - x0) as usize, (y - y0) as usize);
        for py in 0..l.h as usize {
            for px in 0..l.w as usize {
                let s = (py * l.w as usize + px) * 4;
                let sa = l.px[s + 3] as f32 / 255.0;
                if sa <= 0.0 {
                    continue;
                }
                let d = ((oy + py) * w as usize + ox + px) * 4;
                let da = out[d + 3] as f32 / 255.0;
                let oa = sa + da * (1.0 - sa);
                for c in 0..3 {
                    let v = (l.px[s + c] as f32 * sa + out[d + c] as f32 * da * (1.0 - sa)) / oa;
                    out[d + c] = v.round().clamp(0.0, 255.0) as u8;
                }
                out[d + 3] = (oa * 255.0).round() as u8;
            }
        }
    }
    Some((w, h, out))
}

fn png_data_url(w: u32, h: u32, px: &[u8]) -> Result<String, String> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(px).map_err(|e| e.to_string())?;
    }
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&buf)))
}

/* ---------------- 指令 ---------------- */

#[derive(Deserialize, Clone, Default)]
pub struct PartReq {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub mix: Option<String>,
    #[serde(default)]
    pub rate: Option<f32>,
    /// "M" 或 "F"：同名造型有男女兩種時用來挑選
    #[serde(default)]
    pub gender: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct RenderReq {
    #[serde(default)]
    pub game_dir: Option<String>,
    #[serde(default)]
    pub skin: Option<String>,
    #[serde(default)]
    pub hair: Option<PartReq>,
    #[serde(default)]
    pub face: Option<PartReq>,
    /// 角色性別（沒指定髮型、臉型時用來挑預設造型）
    #[serde(default)]
    pub gender: Option<String>,
    /// "full"（全身）或 "head"（只有頭）
    #[serde(default)]
    pub mode: Option<String>,
    /// 點裝名稱（API 目前外觀套組）
    #[serde(default)]
    pub equips: Vec<String>,
    /// 直接指定點裝編號（變更點裝後使用），有值時忽略 equips
    #[serde(default)]
    pub equip_ids: Option<Vec<u32>>,
    /// 新格式：每件點裝可帶染色；有值時忽略 equips 與 equip_ids
    #[serde(default)]
    pub items: Option<Vec<EquipReq>>,
    /// 指定膚色編號（12000 起），優先於 skin 名稱
    #[serde(default)]
    pub skin_id: Option<u32>,
}

#[derive(Serialize)]
pub struct RenderOut {
    pub png: String,
    pub w: u32,
    pub h: u32,
    pub hair_id: Option<u32>,
    pub face_id: Option<u32>,
    pub warnings: Vec<String>,
    /// 實際穿上的點裝
    pub equips: Vec<ItemOut>,
}

#[derive(Serialize)]
pub struct WzStatus {
    pub ok: bool,
    pub path: String,
    pub message: String,
}

fn with_ctx<T>(hint: Option<&str>, f: impl FnOnce(&mut Ctx) -> Result<T, String>) -> Result<T, String> {
    let dir = find_game_dir(hint).ok_or("找不到新楓之谷安裝資料夾，請到「解析」頁選擇遊戲資料夾")?;
    let mut guard = CTX.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().map(|c| c.dir != dir).unwrap_or(true) {
        *guard = None;
        *guard = Some(open_ctx(dir)?);
    }
    f(guard.as_mut().unwrap())
}

fn render_blocking(req: RenderReq) -> Result<RenderOut, String> {
    with_ctx(req.game_dir.as_deref(), |ctx| {
        let mut warnings = Vec::new();
        let female = req.gender.as_deref() == Some("F");
        let head_only = req.mode.as_deref() == Some("head");

        // 膚色：精靈皮膚 → 12012 → 身體 2012、頭 12012
        let skin = req
            .skin_id
            .map(|id| id % 1000)
            .or_else(|| req.skin.as_deref().and_then(|s| ctx.skin.iter().find(|(_, n)| n == s.trim()).map(|(id, _)| id % 1000)))
            .unwrap_or(0);
        let (body_id, head_id) = if ctx.has("Character", 2000 + skin) { (2000 + skin, 12000 + skin) } else { (2000, 12000) };

        // 髮型
        let mut hair_id = None;
        let mut hair_mix = None;
        let hreq = req.hair.clone();
        if let Some(h) = &hreq {
            let fem = h.gender.as_deref().map(|g| g == "F").unwrap_or(female);
            let found = if h.name.trim().is_empty() {
                // 沒指定造型、只指定顏色：用預設髮型
                [if fem { 31000 } else { 30000 }, 30000].into_iter().find(|&id| ctx.has("Character/Hair", id)).map(|id| (id, id))
            } else {
                pick_family(&ctx.hair, &h.name, |id| id / 10 * 10, fem, true)
            };
            match found {
                Some((fam, id)) => {
                    let mut chosen = id;
                    if let Some(c) = h.color.as_deref().and_then(hair_color_idx) {
                        if ctx.has("Character/Hair", fam + c) {
                            chosen = fam + c;
                        }
                    }
                    hair_id = Some(chosen);
                    if let (Some(m), Some(r)) = (h.mix.as_deref().and_then(hair_color_idx), h.rate) {
                        hair_mix = Some((fam + m, r));
                    }
                }
                None => warnings.push(format!("遊戲資料裡找不到髮型「{}」", h.name)),
            }
        } else {
            hair_id = [if female { 31000 } else { 30000 }, 30000].into_iter().find(|&id| ctx.has("Character/Hair", id));
        }

        // 臉型
        let mut face_id = None;
        let mut face_mix = None;
        let face_fam = |id: u32| id - (id / 100 % 10) * 100;
        let freq = req.face.clone();
        if let Some(f) = &freq {
            let fem = f.gender.as_deref().map(|g| g == "F").unwrap_or(female);
            let found = if f.name.trim().is_empty() {
                [if fem { 21000 } else { 20000 }, 20000].into_iter().find(|&id| ctx.has("Character/Face", id)).map(|id| (id, id))
            } else {
                pick_family(&ctx.face, &f.name, face_fam, fem, false)
            };
            match found {
                Some((fam, id)) => {
                    let mut chosen = id;
                    if let Some(c) = f.color.as_deref().and_then(face_color_idx) {
                        if ctx.has("Character/Face", fam + c * 100) {
                            chosen = fam + c * 100;
                        }
                    }
                    face_id = Some(chosen);
                    if let (Some(m), Some(r)) = (f.mix.as_deref().and_then(face_color_idx), f.rate) {
                        face_mix = Some((fam + m * 100, r));
                    }
                }
                None => warnings.push(format!("遊戲資料裡找不到臉型「{}」", f.name)),
            }
        } else {
            face_id = [if female { 21000 } else { 20000 }, 20000].into_iter().find(|&id| ctx.has("Character/Face", id));
        }

        // 點裝：先找出編號，武器決定站姿（雙手武器用 stand2）
        let mut equips: Vec<(u32, &'static str, Option<Dye>)> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        if !head_only {
            let reqs: Vec<EquipReq> = if let Some(items) = &req.items {
                items.clone()
            } else if let Some(ids) = &req.equip_ids {
                ids.iter().map(|&id| EquipReq { id: Some(id), ..Default::default() }).collect()
            } else {
                req.equips.iter().map(|n| EquipReq { name: Some(n.clone()), ..Default::default() }).collect()
            };
            for e in reqs {
                let found = match (e.id, e.name.as_deref()) {
                    (Some(id), _) => ctx.equip_by_id(id),
                    (None, Some(n)) => ctx.find_equip(n, female),
                    _ => None,
                };
                match found {
                    Some((id, cat)) => equips.push((id, cat, e.dye.filter(|d| !d.is_noop()))),
                    None => missing.push(e.name.clone().or(e.id.map(|i| i.to_string())).unwrap_or_default()),
                }
            }
        }
        let equip_out: Vec<ItemOut> = equips
            .iter()
            .map(|&(id, cat, dye)| ItemOut { id, name: ctx.equip_name(id), slot: slot_key(id, cat), cash: true, dye })
            .collect();
        let mut pose = "stand1";
        if let Some(&(wid, _, _)) = equips.iter().find(|(_, c, _)| *c == "Weapon") {
            let root = ctx.data.img("Character/Weapon", &format!("{wid:08}.img")).ok();
            if let Some(root) = root {
                let mut base = root.as_ref();
                if base.get("stand1").is_none() && base.get("stand2").is_none() {
                    if let Some((_, sub)) = root.children().iter().find(|(k, n)| k.chars().all(|c| c.is_ascii_digit()) && n.get("stand1").or(n.get("stand2")).is_some()) {
                        base = sub;
                    }
                }
                if base.get("stand1").is_none() && base.get("stand2").is_some() {
                    pose = "stand2";
                }
            }
        }

        let mut parts: Vec<Layer> = Vec::new();
        let mut has_head = false;
        if !head_only {
            match ctx.layers("Character", body_id, &format!("{pose}/0"), &[], skin) {
                Ok(l) => parts.extend(l),
                Err(e) => warnings.push(format!("身體：{e}")),
            }
        }
        let mut head = ctx.layers("Character", head_id, "front", &["head", "humanEar"], skin).unwrap_or_default();
        if head.is_empty() {
            head = match ctx.layers("Character", head_id, "stand1/0", &["head"], skin) {
                Ok(l) => l,
                Err(e) => {
                    warnings.push(format!("頭：{e}"));
                    Vec::new()
                }
            };
        }
        if !head.is_empty() {
            has_head = true;
            parts.extend(head);
        }
        if let Some(id) = face_id {
            match ctx.mixed("Character/Face", id, face_mix, "default", skin) {
                Ok(l) => parts.extend(l.into_iter().map(|mut l| {
                    l.owner = 1;
                    l
                })),
                Err(e) => warnings.push(format!("臉型：{e}")),
            }
        }
        if let Some(id) = hair_id {
            match ctx.mixed("Character/Hair", id, hair_mix, "default", skin) {
                Ok(l) => parts.extend(l.into_iter().filter(|l| !(head_only && l.name == "hairBelowBody")).map(|mut l| {
                    l.owner = 2;
                    l
                })),
                Err(e) => warnings.push(format!("髮型：{e}")),
            }
        }

        // 點裝圖層；帽子、面具等會依 vslot 蓋掉部分髮型或臉型
        let mut covered: Vec<String> = Vec::new();
        for (i, (id, cat, dye)) in equips.iter().enumerate() {
            let rel = format!("Character/{cat}");
            let Some(frame) = ctx.equip_frame(&rel, *id, pose) else { continue };
            match ctx.layers(&rel, *id, &frame, &[], skin) {
                Ok(mut l) if !l.is_empty() => {
                    if let Some(d) = dye {
                        for layer in l.iter_mut() {
                            apply_dye(&mut layer.px, d);
                        }
                    }
                    let vs = ctx.vslot(&rel, *id);
                    if matches!(*cat, "Cap" | "Accessory" | "Longcoat" | "Coat" | "Cape") {
                        covered.extend(vs.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()));
                    }
                    parts.extend(l.into_iter().map(|mut l| {
                        l.owner = 10 + i;
                        l
                    }));
                }
                Ok(_) => {}
                Err(e) => warnings.push(format!("點裝 {id}：{e}")),
            }
        }
        if !covered.is_empty() {
            let smap = &ctx.smap;
            parts.retain(|l| {
                if l.owner != 1 && l.owner != 2 {
                    return true;
                }
                let codes = smap.get(&l.zname).map(String::as_str).unwrap_or("");
                !codes.as_bytes().chunks(2).any(|c| covered.iter().any(|v| v.as_bytes() == c))
            });
        }
        if !missing.is_empty() {
            warnings.push(format!("遊戲資料裡找不到點裝：{}", missing.join("、")));
        }

        let start: &[(&str, (i32, i32))] = if !head_only && parts.iter().any(|l| l.map.iter().any(|(k, _)| k == "navel")) {
            &[("navel", (0, 0))]
        } else if has_head {
            &[("neck", (0, 0))]
        } else {
            &[("brow", (0, 0))]
        };
        let (w, h, px) = compose(&parts, start).ok_or("沒有可以顯示的圖片")?;
        Ok(RenderOut { png: png_data_url(w, h, &px)?, w, h, hair_id, face_id, warnings, equips: equip_out })
    })
}

#[tauri::command]
pub async fn beauty_render(req: RenderReq) -> Result<RenderOut, String> {
    tauri::async_runtime::spawn_blocking(move || render_blocking(req)).await.map_err(|e| e.to_string())?
}

/// 某個欄位的所有點裝（只列出遊戲資料裡真的有圖的）
#[tauri::command]
pub async fn beauty_items(game_dir: Option<String>, slot: String) -> Result<Vec<ItemOut>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_ctx(game_dir.as_deref(), |ctx| items_of(ctx, &slot))
    })
    .await
    .map_err(|e| e.to_string())?
}

const ALL_SLOTS: [&str; 12] = ["Cap", "Face", "Eye", "Ear", "Coat", "Longcoat", "Pants", "Shoes", "Glove", "Cape", "Shield", "Weapon"];

fn items_of(ctx: &mut Ctx, slot: &str) -> Result<Vec<ItemOut>, String> {
    if let Some(v) = ctx.lists.get(slot) {
        return Ok(v.clone());
    }
    if slot == "All" {
        let mut all = Vec::new();
        for s in ALL_SLOTS {
            // 某個分類讀不到（例如資料夾不存在）就略過，不影響其他分類
            if let Ok(v) = items_of(ctx, s) {
                all.extend(v);
            }
        }
        ctx.lists.insert(slot.to_string(), all.clone());
        return Ok(all);
    }
    let slot = slot.to_string();
    let (cat, prefix) = slot_cat(&slot).ok_or("未知的欄位")?;
    let rel = format!("Character/{cat}");
    let pkg = ctx.data.package(&rel)?;
    let mut seen = std::collections::HashSet::new();
    let cands: Vec<(u32, String, &'static str)> = ctx
        .equip
        .iter()
        .filter(|(id, _, c)| *c == cat && prefix.map(|p| id / 10000 == p).unwrap_or(true))
        .filter(|(id, _, _)| pkg.imgs.contains_key(&format!("{id:08}.img")))
        .filter(|(id, _, _)| seen.insert(*id))
        .cloned()
        .collect();
    let mut list = Vec::with_capacity(cands.len());
    for (id, name, c) in cands {
        let cash = ctx
            .data
            .img_child(&rel, &format!("{id:08}.img"), "info")
            .ok()
            .flatten()
            .and_then(|info| info.get("cash").and_then(|v| v.as_int()))
            .map(|v| v == 1)
            .unwrap_or(false);
        list.push(ItemOut { id, name, slot: slot_key(id, c), cash, dye: None });
    }
    ctx.lists.insert(slot, list.clone());
    Ok(list)
}

#[derive(Serialize)]
pub struct SkinOut {
    pub id: u32,
    pub name: String,
    /// 頭部圖片的平均膚色，用來畫色塊
    pub color: String,
}

/// 遊戲資料裡有身體圖的膚色
#[tauri::command]
pub async fn beauty_skins(game_dir: Option<String>) -> Result<Vec<SkinOut>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_ctx(game_dir.as_deref(), |ctx| {
            let list = ctx.skin.clone();
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (id, name) in list {
                let s = id % 1000;
                if !seen.insert(s) || !ctx.has("Character", 2000 + s) {
                    continue;
                }
                let color = ctx
                    .layers("Character", 12000 + s, "front", &["head"], s)
                    .ok()
                    .and_then(|l| l.into_iter().next())
                    .map(|l| {
                        // 只取不透明、不太暗的像素（排除外框線）
                        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
                        for p in l.px.chunks_exact(4) {
                            if p[3] > 200 && (p[0] as u32 + p[1] as u32 + p[2] as u32) > 200 {
                                r += p[0] as u64;
                                g += p[1] as u64;
                                b += p[2] as u64;
                                n += 1;
                            }
                        }
                        if n == 0 { String::new() } else { format!("#{:02x}{:02x}{:02x}", r / n, g / n, b / n) }
                    })
                    .unwrap_or_default();
                out.push(SkinOut { id, name, color });
            }
            Ok(out)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 道具圖示（PNG data URL），找不到的回傳 null
#[tauri::command]
pub async fn beauty_icons(game_dir: Option<String>, ids: Vec<u32>) -> Result<Vec<Option<String>>, String> {
    tauri::async_runtime::spawn_blocking(move || with_ctx(game_dir.as_deref(), |ctx| Ok(ids.iter().map(|&id| ctx.icon(id)).collect())))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn beauty_wz_status(game_dir: Option<String>) -> WzStatus {
    tauri::async_runtime::spawn_blocking(move || {
        match with_ctx(game_dir.as_deref(), |ctx| {
            Ok((ctx.dir.display().to_string(), ctx.hair.len(), ctx.face.len()))
        }) {
            Ok((path, h, f)) => WzStatus { ok: true, path, message: format!("已讀取：髮型 {h} 種、臉型 {f} 種") },
            Err(e) => WzStatus { ok: false, path: String::new(), message: e },
        }
    })
    .await
    .unwrap_or(WzStatus { ok: false, path: String::new(), message: "讀取失敗".into() })
}

#[tauri::command]
pub async fn pick_game_dir() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("選擇新楓之谷安裝資料夾")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(hair_color_idx("黑色"), Some(0));
        assert_eq!(hair_color_idx("褐色"), Some(7));
        assert_eq!(face_color_idx("紫色"), Some(6));
        assert_eq!(face_color_idx("祖母綠"), Some(5));
        assert_eq!(face_color_idx("紫水晶"), Some(7));
        assert_eq!(face_color_idx("綠色"), Some(3));
        assert_eq!(strip_hair_color("黑色餘韻髮型"), "餘韻髮型");
    }

    #[test]
    fn family_pick() {
        let list = vec![(57047, "神經藥臉部".to_string()), (57147, "神經藥臉部".to_string()), (58080, "神經藥臉部".to_string())];
        let fam = |id: u32| id - (id / 100 % 10) * 100;
        assert_eq!(pick_family(&list, "神經藥臉部", fam, false, false), Some((57047, 57047)));
        assert_eq!(pick_family(&list, "神經藥臉部", fam, true, false), Some((58080, 58080)));
        let hair = vec![(66790, "黑色餘韻髮型".to_string()), (66791, "紅色餘韻髮型".to_string())];
        assert_eq!(pick_family(&hair, "藍色餘韻髮型", |id| id / 10 * 10, true, true).map(|x| x.0), Some(66790));
    }

    #[test]
    fn dye_hsv() {
        let (h, s2, v) = rgb_to_hsv(1.0, 0.0, 0.0);
        assert!((h - 0.0).abs() < 1e-3 && (s2 - 1.0).abs() < 1e-3 && (v - 1.0).abs() < 1e-3);
        let (r, g, b) = hsv_to_rgb(120.0, 1.0, 1.0);
        assert!(r < 1e-3 && (g - 1.0).abs() < 1e-3 && b < 1e-3);
        // 紅色轉 120 度變綠色；藍色不在紅色系範圍內，不受影響
        let mut px = vec![255, 0, 0, 255, 0, 0, 255, 255];
        apply_dye(&mut px, &Dye { range: 1, hue: 120, sat: 0, val: 0 });
        assert_eq!(&px[0..3], &[0, 255, 0]);
        assert_eq!(&px[4..7], &[0, 0, 255]);
    }

    #[test]
    fn resolve_uol() {
        let root = Node::Props(vec![
            ("front".into(), Node::Props(vec![("head".into(), Node::Int(7))])),
            ("stand1".into(), Node::Props(vec![("0".into(), Node::Props(vec![("head".into(), Node::Uol("../../front/head".into()))]))])),
        ]);
        assert!(matches!(resolve(&root, "stand1/0/head", 0), Some(Node::Int(7))));
    }
}
