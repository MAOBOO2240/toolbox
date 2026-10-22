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
    /// 髮型／臉型清單快取（每個造型一筆，不分顏色）
    styles: HashMap<String, Vec<StyleOut>>,
    /// 所有道具名稱 → (資料夾, 編號)；第一次查道具圖示時才建立
    item_names: Option<HashMap<String, Vec<(&'static str, u32)>>>,
    /// 依名稱查到的道具圖示
    name_icons: HashMap<String, Option<String>>,
    /// 某個 img 實際所在的分類資料夾
    img_rels: HashMap<String, Option<String>>,
    /// 道具特效（ItemEff.img 的子項目）快取
    eff_nodes: HashMap<u32, Option<(String, std::sync::Arc<Node>)>>,
    /// 戒指：編號、名稱
    rings: Option<Vec<(u32, String)>>,
    /// 戒指的名牌編號、對話框編號、是否為點裝
    ring_info: HashMap<u32, (Option<i64>, Option<i64>, bool)>,
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
        "Ring" => ("Ring", None),
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
    let h = h.rem_euclid(360.0);
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match (h as u32) / 60 {
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

/// 染色稜鏡的色系範圍（色相取整數後比較；紅色系不含灰階）
fn prism_in_range(range: u8, hue: f32, sat: f32) -> bool {
    let h = hue as i32;
    match range {
        1 => !((30..=330).contains(&h) || sat == 0.0),
        2 => (30..=90).contains(&h),
        3 => (90..=150).contains(&h),
        4 => (150..=210).contains(&h),
        5 => (210..=270).contains(&h),
        6 => (270..=330).contains(&h),
        _ => true,
    }
}

/// 四捨五入的方式和遊戲一致：小數部分要到 0.985 才進位
fn prism_round(value: f32, step: f32) -> u8 {
    let q = value / step;
    let r = if q - q.floor() >= 0.985 { q.ceil() } else { q.floor() };
    (r * step).clamp(0.0, 255.0) as u8
}

/// 染色稜鏡：和遊戲相同的演算法（參考 WzComparerR2 AvatarCommon/Prism.cs，MIT 授權）。
/// API 的飽和度、亮度是 -99～99，遊戲內部以 100 為不變（1～199）。
/// 純黑、純白、透明的點不染；4 位元色（ARGB4444）的圖，結果要對齊 17 的倍數。
fn apply_dye(px: &mut [u8], dye: &Dye) {
    apply_dye_ex(px, dye, false)
}

/// effect = true 時（道具特效）不對齊 17 的倍數
fn apply_dye_ex(px: &mut [u8], dye: &Dye, effect: bool) {
    let hue = dye.hue.rem_euclid(360) as f32;
    let sat = (100 + dye.sat.clamp(-99, 99)) as f32;
    let bri = (100 + dye.val.clamp(-99, 99)) as f32;
    if dye.range > 6 || (hue == 0.0 && sat == 100.0 && bri == 100.0) {
        return;
    }
    let step = if effect || px.chunks_exact(4).any(|p| p[0] % 17 != 0 || p[1] % 17 != 0 || p[2] % 17 != 0) { 1.0 } else { 17.0 };
    for p in px.chunks_exact_mut(4) {
        let (r8, g8, b8, a) = (p[0], p[1], p[2], p[3]);
        if a == 0 || (r8 == 0 && g8 == 0 && b8 == 0) || (r8 == 255 && g8 == 255 && b8 == 255) {
            continue;
        }
        let (h, s, v) = rgb_to_hsv(r8 as f32 / 255.0, g8 as f32 / 255.0, b8 as f32 / 255.0);
        let h = if r8 == g8 && g8 == b8 { 0.0 } else { h };
        if !prism_in_range(dye.range, h, s) {
            continue;
        }
        // 色相
        let h2 = (h + hue) % 360.0;
        // 飽和度（也會影響亮度）
        let k = (sat - 100.0) / 100.0;
        let (ds, dv) = if sat < 100.0 {
            (k * s, k * s * v / 2.0)
        } else if s > 0.0 && v < 1.0 {
            (k * (2.0 - s) * (1.0 - v), k * (1.0 - s) * v)
        } else {
            (0.0, 0.0)
        };
        let ts = (s + ds).clamp(0.0, 1.0);
        let tv = (v + dv).clamp(0.0, 1.0);
        // 亮度（也會影響飽和度）
        let kb = (bri - 100.0) / 100.0;
        let (ds2, dv2) = if bri < 100.0 { (0.0, kb * s * tv) } else { (kb * -ts, kb * (15.0 - 12.0 * s) / 15.0 * (1.0 - tv)) };
        let s2 = (ts + ds2).clamp(0.0, 1.0);
        let v2 = (tv + dv2).clamp(0.0, 1.0);
        let (r, g, b) = hsv_to_rgb(h2, s2, v2);
        p[0] = prism_round(r * 255.0, step);
        p[1] = prism_round(g * 255.0, step);
        p[2] = prism_round(b * 255.0, step);
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

const EQUIP_CATS: [&str; 11] = ["Cap", "Accessory", "Coat", "Longcoat", "Pants", "Shoes", "Glove", "Cape", "Shield", "Weapon", "Ring"];

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

/* ---------------- 閒置時放開遊戲檔 ----------------
 * 讀好的遊戲資料（和開著的遊戲檔）在一段時間沒用之後放開，要用時再重新開；
 * 工具箱縮在右下角時不會一直佔著遊戲檔與記憶體，遊戲更新也不會被擋 */
const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(180);
static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// 最後一次用到遊戲資料的時間（從 EPOCH 起算的毫秒，0 = 還沒用過）
static LAST_USE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}

fn touch() {
    LAST_USE.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    static WATCH: std::sync::Once = std::sync::Once::new();
    WATCH.call_once(|| {
        let _ = std::thread::Builder::new().name("wz-idle".into()).spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(20));
            let idle = |last: u64| last != 0 && now_ms().saturating_sub(last) >= IDLE_RELEASE.as_millis() as u64;
            if !idle(LAST_USE.load(std::sync::atomic::Ordering::Relaxed)) {
                continue;
            }
            // 正在使用中（拿不到鎖）就等下一輪
            if let Ok(mut g) = CTX.try_lock() {
                if g.is_some() && idle(LAST_USE.load(std::sync::atomic::Ordering::Relaxed)) {
                    *g = None;
                }
            }
        });
    });
}

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
        styles: HashMap::new(),
        item_names: None,
        name_icons: HashMap::new(),
        img_rels: HashMap::new(),
        eff_nodes: HashMap::new(),
        rings: None,
        ring_info: HashMap::new(),
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
            icon_png(w, h, &px)
        })();
        self.icons.insert(id, v.clone());
        v
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
/// 回傳 (寬, 高, 像素, 起始錨點在圖中的 x, y)
fn compose(parts: &[Layer], start: &[(&str, (i32, i32))]) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
    compose_full(parts, start).map(|c| (c.w, c.h, c.px, -c.x0, -c.y0))
}

struct Composed {
    w: u32,
    h: u32,
    px: Vec<u8>,
    /// 圖片左上角在組合座標中的位置
    x0: i32,
    y0: i32,
    /// 各錨點在組合座標中的位置；「@root」是身體圖的原點（角色腳下）
    anchors: HashMap<String, (i32, i32)>,
}

fn compose_full(parts: &[Layer], start: &[(&str, (i32, i32))]) -> Option<Composed> {
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
        // 身體圖的原點就是角色站的位置，特效、殘影以這裡為準
        if l.name == "body" {
            anchors.entry("@root".to_string()).or_insert(pos);
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
    Some(Composed { w, h, px: out, x0, y0, anchors })
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
    #[serde(default)]
    pub name: String,
    /// 直接指定編號（髮型／臉型視窗選的），有值時不用名稱找
    #[serde(default)]
    pub id: Option<u32>,
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
    /// 要播放的動作："stand"、"attack"、"walk"、"ladder"、"prone"、"sit"、"jump"；
    /// 有值時另外輸出每一格畫面（anim）
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Serialize)]
pub struct RenderOut {
    pub png: String,
    pub w: u32,
    pub h: u32,
    pub hair_id: Option<u32>,
    pub face_id: Option<u32>,
    pub warnings: Vec<String>,
    /// 角色中心在圖中的位置
    pub cx: i32,
    pub cy: i32,
    /// 角色腳下（身體原點）在圖中的位置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fx: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fy: Option<i32>,
    /// 實際穿上的點裝
    pub equips: Vec<ItemOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anim: Option<AnimOut>,
}

/// 一格畫面；x、y 是圖片左上角相對於角色腳下（或特效的錨點）的位置
#[derive(Serialize, Clone)]
pub struct AnimFrame {
    pub png: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub delay: i32,
    /// brow、neck、navel 相對於角色腳下的位置（只有角色畫面有）
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub anchors: HashMap<String, (i32, i32)>,
}

/// 道具特效（Effect/ItemEff.img）：自己循環播放，z >= 0 畫在角色前面，< 0 畫在後面
#[derive(Serialize)]
pub struct EffTrack {
    pub z: i32,
    /// 跟著哪個錨點："" = 角色腳下，或 brow、neck、navel
    pub anchor: String,
    pub frames: Vec<AnimFrame>,
}

#[derive(Serialize)]
pub struct AnimOut {
    /// 實際使用的動作名稱（例如 stand1、swingO1）
    pub action: String,
    pub frames: Vec<AnimFrame>,
    pub effects: Vec<EffTrack>,
    /// 站立時角色中心相對於腳下的位置（切換動作時畫面不會跳動）
    pub center: (i32, i32),
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
    touch();
    if guard.as_ref().map(|c| c.dir != dir).unwrap_or(true) {
        *guard = None;
        *guard = Some(open_ctx(dir)?);
    }
    let out = f(guard.as_mut().unwrap());
    touch();
    out
}

/// 解析好的角色外觀，畫每一格動作時共用
struct Look {
    body_id: u32,
    head_id: u32,
    skin: u32,
    hair_id: Option<u32>,
    hair_mix: Option<(u32, f32)>,
    face_id: Option<u32>,
    face_mix: Option<(u32, f32)>,
    equips: Vec<(u32, &'static str, Option<Dye>)>,
    /// 點裝武器依武器種類分的子資料夾（例如 "30/"），一般武器是空字串
    wprefix: String,
    /// 武器種類（30 單手劍、40 雙手劍…），決定普通攻擊的動作
    wtype: Option<u32>,
    /// 站姿：stand1，雙手武器是 stand2
    pose: &'static str,
    /// 帽子、套服等蓋住的欄位代碼（vslot）
    covered: Vec<String>,
    /// 帽子蓋住頭髮的程度（依帽子的 vslot，和 HeavenClient 相同）
    cap: CapType,
}

/// 普通攻擊：依武器種類挑動作（找不到時改用武器有的其他攻擊動作）
fn attack_prefs(wtype: Option<u32>) -> &'static [&'static str] {
    match wtype {
        Some(40..=42) => &["swingT1", "swingT3", "swingT2", "stabT1"],
        Some(43) => &["stabT1", "stabT2", "swingT2"],
        Some(44) => &["swingP1", "swingP2", "stabT2"],
        Some(45) => &["shoot1"],
        Some(46) => &["shoot2"],
        Some(49) => &["shootF", "shoot1"],
        _ => &["swingO1", "stabO1", "swingO2", "swingO3"],
    }
}

const ATTACK_ACTIONS: [&str; 19] = [
    "swingO1", "swingO2", "swingO3", "swingOF", "swingT1", "swingT2", "swingT3", "swingTF", "swingP1", "swingP2", "swingPF", "stabO1",
    "stabO2", "stabOF", "stabT1", "stabT2", "stabTF", "shoot1", "shoot2",
];

/// 武器沒有寫殘影名稱時（點裝武器），依武器種類猜一個
fn afterimage_guess(wtype: Option<u32>) -> &'static [&'static str] {
    match wtype {
        Some(30) => &["swordOL", "swordOS"],
        Some(31) | Some(41) => &["axe"],
        Some(32) | Some(42) | Some(37) | Some(38) => &["mace"],
        Some(33) => &["dagger"],
        Some(40) => &["swordTL", "swordTS"],
        Some(43) => &["spear"],
        Some(44) => &["poleArm"],
        Some(45) => &["bow"],
        Some(46) => &["crossBow"],
        Some(47) => &["claw"],
        Some(48) => &["knuckle"],
        Some(49) => &["gun"],
        _ => &[],
    }
}

/// 一張圖：寬、高、像素、原點
type Pic = (u32, u32, Vec<u8>, (i32, i32));

/// 把幾張各自有原點的圖合成一張；回傳 (寬, 高, 像素, 左上角相對於原點的 x, y)
fn merge_canvases(list: &[Pic]) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
    let x0 = list.iter().map(|(_, _, _, o)| -o.0).min()?;
    let y0 = list.iter().map(|(_, _, _, o)| -o.1).min()?;
    let x1 = list.iter().map(|(w, _, _, o)| -o.0 + *w as i32).max()?;
    let y1 = list.iter().map(|(_, h, _, o)| -o.1 + *h as i32).max()?;
    let (w, h) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
    if w > 2000 || h > 2000 {
        return None;
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (lw, lh, px, o) in list {
        let (ox, oy) = ((-o.0 - x0) as usize, (-o.1 - y0) as usize);
        for py in 0..*lh as usize {
            for pxx in 0..*lw as usize {
                let s = (py * *lw as usize + pxx) * 4;
                let sa = px[s + 3] as f32 / 255.0;
                if sa <= 0.0 {
                    continue;
                }
                let d = ((oy + py) * w as usize + ox + pxx) * 4;
                let da = out[d + 3] as f32 / 255.0;
                let oa = sa + da * (1.0 - sa);
                for c in 0..3 {
                    out[d + c] = ((px[s + c] as f32 * sa + out[d + c] as f32 * da * (1.0 - sa)) / oa).round().clamp(0.0, 255.0) as u8;
                }
                out[d + 3] = (oa * 255.0).round() as u8;
            }
        }
    }
    Some((w, h, out, x0, y0))
}

fn canvas_origin(node: &Node) -> (i32, i32) {
    node.get("origin").and_then(|v| v.as_vec()).unwrap_or((0, 0))
}

fn canvas_delay(node: &Node) -> Option<i32> {
    node.get("delay").and_then(|v| v.as_int()).map(|d| (d as i32).abs()).filter(|d| *d > 0)
}

impl Ctx {
    /// 第一個存在的畫格路徑
    fn first_frame(&mut self, rel: &str, id: u32, cands: &[String]) -> Option<String> {
        let root = self.data.img(rel, &format!("{id:08}.img")).ok()?;
        cands.iter().find(|c| resolve(&root, c, 0).is_some()).cloned()
    }

    /// 裝備在某一格動作要用的畫格：一般是「動作/格」；點裝武器多一層武器種類；
    /// 帽子、飾品沒有動作時用 default（背面用 backDefault）；臉飾、眼飾在背面時不顯示
    #[allow(clippy::too_many_arguments)]
    fn equip_frame_at(&mut self, rel: &str, id: u32, cat: &str, prefix: &str, action: &str, idx: usize, face: bool) -> Option<String> {
        let acc = if cat == "Accessory" { id / 10000 } else { 0 };
        if !face && (acc == 101 || acc == 102) {
            return None;
        }
        let mut cands: Vec<String> = Vec::new();
        if acc == 101 {
            cands.push("default".into());
        }
        cands.push(format!("{prefix}{action}/{idx}"));
        cands.push(format!("{prefix}{action}/0"));
        if cat == "Cap" || cat == "Accessory" {
            if !face {
                cands.push("backDefault".into());
            }
            cands.push("default".into());
        } else if action.starts_with("stand") {
            cands.extend([format!("{prefix}stand1/0"), format!("{prefix}stand2/0"), "default".into()]);
        } else {
            cands.push("default".into());
        }
        self.first_frame(rel, id, &cands)
    }

    /// 某個 img 在分類資料夾或它底下一層子資料夾的位置（例如 Effect/ItemEff.img、UI/NameTag.img）
    fn img_rel(&mut self, base: &str, img: &str) -> Option<String> {
        let key = format!("{base}|{img}");
        if let Some(v) = self.img_rels.get(&key) {
            return v.clone();
        }
        let found = (|| {
            let p = self.data.package(base).ok()?;
            if p.imgs.contains_key(img) {
                return Some(base.to_string());
            }
            for sd in p.subdirs.iter().filter(|s| !s.starts_with('_')) {
                let rel = format!("{base}/{sd}");
                if let Ok(q) = self.data.package(&rel) {
                    if q.imgs.contains_key(img) {
                        return Some(rel);
                    }
                }
            }
            None
        })();
        self.img_rels.insert(key, found.clone());
        found
    }

    /// 只解析大 img 的其中一個子項目（ItemEff.img 很大，整個解析太慢）；_inlink 也在這個子項目裡找
    fn child_pixels(&mut self, rel: &str, img: &str, key: &str, child: &Node, node: &Node, depth: u32) -> Option<(u32, u32, Vec<u8>)> {
        if depth > 4 {
            return None;
        }
        if let Node::Canvas(_, props) = node {
            if let Some(link) = props.iter().find(|(k, _)| k == "_inlink").and_then(|(_, v)| v.as_str()) {
                let rest = link.strip_prefix(&format!("{key}/"))?;
                let target = resolve(child, rest, 0)?.clone();
                return self.child_pixels(rel, img, key, child, &target, depth + 1);
            }
        }
        self.data.pixels_of(rel, img, node).ok()
    }

    /// ItemEff.img 裡這個道具的節點（查過就記住）
    fn eff_node(&mut self, id: u32) -> Option<(String, std::sync::Arc<Node>)> {
        let rel = self.img_rel("Effect", "ItemEff.img")?;
        match self.eff_nodes.get(&id) {
            Some(v) => v.clone(),
            None => {
                let mut got = None;
                for key in [id.to_string(), format!("{id:08}")] {
                    if let Ok(Some(n)) = self.data.img_child(&rel, "ItemEff.img", &key) {
                        got = Some((key, std::sync::Arc::new(n)));
                        break;
                    }
                }
                self.eff_nodes.insert(id, got.clone());
                got
            }
        }
    }

    /// 這件道具有沒有自己的攻擊特效（effect 底下有攻擊動作的畫格）
    fn has_attack_effect(&mut self, id: u32, action: &str) -> bool {
        let Some((_, child)) = self.eff_node(id) else { return false };
        if resolve(&child, &format!("effect/{action}/0"), 0).is_some() {
            return true;
        }
        ATTACK_ACTIONS.iter().any(|a| resolve(&child, &format!("effect/{a}/0"), 0).is_some())
    }

    /// 道具特效（Effect/ItemEff.img/編號/effect/動作）
    fn item_effect(&mut self, id: u32, action: &str, dye: Option<Dye>) -> Option<EffTrack> {
        let rel = self.img_rel("Effect", "ItemEff.img")?;
        let img = "ItemEff.img";
        let (key, child) = self.eff_node(id)?;
        child.get("effect")?;
        // 依動作找；沒有就用 default；有些特效直接把畫格放在 effect 底下
        let act_path = [format!("effect/{action}"), "effect/default".to_string(), "effect".to_string()]
            .into_iter()
            .find(|p| resolve(&child, &format!("{p}/0"), 0).is_some())?;
        let act = resolve(&child, &act_path, 0)?.clone();
        let z = act.get("z").and_then(|v| v.as_int()).unwrap_or(-2) as i32;
        let anchor = match act.get("pos").and_then(|v| v.as_int()).unwrap_or(0) {
            1 => "brow",
            2 => "neck",
            4 => "navel",
            _ => "",
        };
        let mut frames = Vec::new();
        for i in 0..90 {
            let Some(fnode) = resolve(&child, &format!("{act_path}/{i}"), 0).cloned() else { break };
            let mut pieces: Vec<Pic> = Vec::new();
            let mut delay = canvas_delay(&fnode);
            let canvases: Vec<Node> = match &fnode {
                Node::Canvas(..) => vec![fnode.clone()],
                Node::Props(kids) => kids
                    .iter()
                    .filter_map(|(k, _)| resolve(&child, &format!("{act_path}/{i}/{k}"), 0).cloned())
                    .filter(|n| matches!(n, Node::Canvas(..)))
                    .collect(),
                _ => Vec::new(),
            };
            for cn in canvases {
                if delay.is_none() {
                    delay = canvas_delay(&cn);
                }
                let origin = canvas_origin(&cn);
                if let Some((w, h, mut px)) = self.child_pixels(&rel, img, &key, &child, &cn, 0) {
                    if w <= 1 && h <= 1 {
                        continue;
                    }
                    if let Some(d) = &dye {
                        apply_dye_ex(&mut px, d, true);
                    }
                    pieces.push((w, h, px, origin));
                }
            }
            let Some((w, h, px, x, y)) = merge_canvases(&pieces) else { continue };
            frames.push(AnimFrame { png: png_data_url(w, h, &px).ok()?, x, y, w, h, delay: delay.unwrap_or(120), anchors: HashMap::new() });
        }
        if frames.is_empty() {
            return None;
        }
        Some(EffTrack { z, anchor: anchor.to_string(), frames })
    }

    /// 攻擊殘影（Character/Afterimage/名稱.img/等級/動作/第幾格）：疊在對應的那一格最上層
    fn afterimage(&mut self, look: &Look, action: &str) -> HashMap<usize, Vec<Layer>> {
        let mut out: HashMap<usize, Vec<Layer>> = HashMap::new();
        let Some(&(wid, _, _)) = look.equips.iter().find(|(_, c, _)| *c == "Weapon") else { return out };
        let rel = "Character/Afterimage";
        let Ok(pkg) = self.data.package(rel) else { return out };
        let mut names: Vec<String> = Vec::new();
        if let Ok(Some(info)) = self.data.img_child("Character/Weapon", &format!("{wid:08}.img"), "info") {
            if let Some(s) = info.get("afterImage").and_then(|v| v.as_str()) {
                names.push(s.to_string());
            }
        }
        names.extend(afterimage_guess(look.wtype).iter().map(|s| s.to_string()));
        let Some(img) = names.iter().map(|n| format!("{n}.img")).find(|n| pkg.imgs.contains_key(n)) else { return out };
        let Ok(root) = self.data.img(rel, &img) else { return out };
        // 依角色等級分段，取最高的一段
        let Some(lvl) = root.children().iter().filter_map(|(k, _)| k.parse::<u32>().ok()).max() else { return out };
        let Some(stance) = resolve(&root, &format!("{lvl}/{action}"), 0).cloned() else { return out };
        let keys: Vec<usize> = stance.children().iter().filter_map(|(k, _)| k.parse().ok()).collect();
        for k in keys {
            let base = format!("{lvl}/{action}/{k}");
            let Some(node) = resolve(&root, &base, 0).cloned() else { continue };
            let list: Vec<(usize, Node)> = match &node {
                Node::Canvas(..) => vec![(k, node.clone())],
                Node::Props(kids) => kids
                    .iter()
                    .filter_map(|(j, _)| j.parse::<usize>().ok())
                    .filter_map(|j| resolve(&root, &format!("{base}/{j}"), 0).cloned().map(|n| (k + j, n)))
                    .filter(|(_, n)| matches!(n, Node::Canvas(..)))
                    .collect(),
                _ => Vec::new(),
            };
            for (fi, cn) in list {
                let origin = canvas_origin(&cn);
                if let Ok((w, h, px)) = self.data.pixels_of(rel, &img, &cn) {
                    if w <= 1 && h <= 1 {
                        continue;
                    }
                    out.entry(fi).or_default().push(Layer {
                        name: "afterimage".into(),
                        zname: "afterimage".into(),
                        owner: 99,
                        z: i32::MIN / 2,
                        origin,
                        map: vec![("@root".into(), (0, 0))],
                        w,
                        h,
                        px,
                    });
                }
            }
        }
        out
    }
}

/// 名稱、髮型、臉型、點裝 → 編號
fn resolve_look(ctx: &mut Ctx, req: &RenderReq, warnings: &mut Vec<String>) -> (Look, Vec<ItemOut>) {
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
    if let Some(h) = &req.hair {
        let fem = h.gender.as_deref().map(|g| g == "F").unwrap_or(female);
        let found = if let Some(id) = h.id.filter(|&id| ctx.has("Character/Hair", id)) {
            Some((id / 10 * 10, id))
        } else if h.name.trim().is_empty() {
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
    if let Some(f) = &req.face {
        let fem = f.gender.as_deref().map(|g| g == "F").unwrap_or(female);
        let found = if let Some(id) = f.id.filter(|&id| ctx.has("Character/Face", id)) {
            Some((face_fam(id), id))
        } else if f.name.trim().is_empty() {
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

    // 點裝
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
    if !missing.is_empty() {
        warnings.push(format!("遊戲資料裡找不到點裝：{}", missing.join("、")));
    }
    // 戒指只留名牌戒指、聊天戒指（其他戒指看不出外觀）
    equips.retain(|&(id, cat, _)| cat != "Ring" || ctx.ring_slot(id) != "Ring");
    let equip_out: Vec<ItemOut> = equips
        .iter()
        .map(|&(id, cat, dye)| {
            let slot = if cat == "Ring" { ctx.ring_slot(id) } else { slot_key(id, cat) };
            ItemOut { id, name: ctx.equip_name(id), slot, cash: true, dye }
        })
        .collect();

    // 武器決定站姿（雙手武器用 stand2）；點裝武器依武器種類分子資料夾
    let mut pose = "stand1";
    let mut wprefix = String::new();
    let mut wtype = None;
    if let Some(&(wid, _, _)) = equips.iter().find(|(_, c, _)| *c == "Weapon") {
        wtype = Some(wid / 10000 % 100);
        if let Ok(root) = ctx.data.img("Character/Weapon", &format!("{wid:08}.img")) {
            let mut base = root.as_ref();
            if base.get("stand1").is_none() && base.get("stand2").is_none() {
                if let Some((k, sub)) = root.children().iter().find(|(k, n)| k.chars().all(|c| c.is_ascii_digit()) && n.get("stand1").or(n.get("stand2")).is_some()) {
                    base = sub;
                    wprefix = format!("{k}/");
                    wtype = k.parse::<u32>().ok().map(|t| t % 100);
                }
            }
            if base.get("stand1").is_none() && base.get("stand2").is_some() {
                pose = "stand2";
            }
        }
    }

    // 帽子、套服等用 vslot 蓋住部分髮型、臉型
    let mut covered: Vec<String> = Vec::new();
    let mut cap = CapType::None;
    for (id, cat, _) in &equips {
        if matches!(*cat, "Cap" | "Accessory" | "Longcoat" | "Coat" | "Cape") {
            let vs = ctx.vslot(&format!("Character/{cat}"), *id);
            if *cat == "Cap" && !vs.is_empty() {
                cap = cap_type(&vs);
            }
            covered.extend(vs.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()));
        }
    }

    (Look { body_id, head_id, skin, hair_id, hair_mix, face_id, face_mix, equips, wprefix, wtype, pose, covered, cap }, equip_out)
}

/// 帽子的種類：沒有、頭帶（CpH5）、蓋一半（CpH1H5）、全蓋（CpH1H5H6）
#[derive(Clone, Copy, PartialEq, Debug)]
enum CapType {
    None,
    Headband,
    Half,
    Full,
}

fn cap_type(vslot: &str) -> CapType {
    let has = |c: &str| vslot.as_bytes().chunks(2).any(|x| x == c.as_bytes());
    if vslot.is_empty() {
        CapType::None
    } else if has("H1") && has("H6") {
        CapType::Full
    } else if has("H1") {
        CapType::Half
    } else if has("H5") {
        CapType::Headband
    } else {
        CapType::None
    }
}

/// 頭髮的圖層要不要畫（和 HeavenClient、WzComparerR2 相同）：
/// - hairBelowBody（身體後面的長髮）一定畫，戴帽子也一樣
/// - 沒帽子／頭帶：hairOverHead、backHair；蓋一半：hair、backHairBelowCap；全蓋：只剩身體後面的長髮
/// - 沒帽子時 hair 只在沒有 hairOverHead 的髮型才畫
fn hair_visible(name: &str, cap: CapType, has_over_head: bool) -> bool {
    match name {
        "hairBelowBody" => true,
        _ if cap == CapType::Full => false,
        "hairOverHead" | "backHair" | "backHairOverCape" => matches!(cap, CapType::None | CapType::Headband),
        "hair" => cap != CapType::None || !has_over_head,
        "backHairBelowCap" => cap == CapType::Half,
        _ => true,
    }
}

/// 某一格動作的所有圖層；回傳 (圖層, 延遲毫秒, 是否看得到臉)
fn frame_parts(ctx: &mut Ctx, look: &Look, action: &str, idx: usize, warnings: &mut Vec<String>) -> Result<(Vec<Layer>, i32, bool), String> {
    let fpath = format!("{action}/{idx}");
    let broot = ctx.data.img("Character", &format!("{:08}.img", look.body_id))?;
    let fnode = resolve(&broot, &fpath, 0).ok_or_else(|| format!("身體沒有 {fpath}"))?;
    let delay = canvas_delay(fnode).unwrap_or(120);
    // 身體畫格的 face = 0 表示背對（爬梯子、繩子）
    let face = fnode.get("face").and_then(|v| v.as_int()).map(|v| v != 0).unwrap_or(!matches!(action, "ladder" | "rope"));

    let mut parts = ctx.layers("Character", look.body_id, &fpath, &[], look.skin)?;
    // 頭：看得到臉用正面，爬梯子時用背面
    let mut head = ctx.layers("Character", look.head_id, if face { "front" } else { "back" }, &["head", "humanEar"], look.skin).unwrap_or_default();
    if head.is_empty() {
        head = ctx.layers("Character", look.head_id, &fpath, &["head"], look.skin).unwrap_or_default();
    }
    if head.is_empty() {
        warnings.push("讀不到頭部圖片".into());
    }
    parts.extend(head);
    if face {
        if let Some(id) = look.face_id {
            match ctx.mixed("Character/Face", id, look.face_mix, "default", look.skin) {
                Ok(l) => parts.extend(l.into_iter().map(|mut l| {
                    l.owner = 1;
                    l
                })),
                Err(e) => warnings.push(format!("臉型：{e}")),
            }
        }
    }
    if let Some(id) = look.hair_id {
        let cands = [fpath.clone(), (if face { "stand1/0" } else { "ladder/0" }).to_string(), (if face { "default" } else { "backDefault" }).to_string()];
        if let Some(fr) = ctx.first_frame("Character/Hair", id, &cands) {
            match ctx.mixed("Character/Hair", id, look.hair_mix, &fr, look.skin) {
                Ok(l) => {
                    let over = l.iter().any(|x| x.name == "hairOverHead");
                    parts.extend(l.into_iter().filter(|l| hair_visible(&l.name, look.cap, over)).map(|mut l| {
                        l.owner = 2;
                        l
                    }))
                }
                Err(e) => warnings.push(format!("髮型：{e}")),
            }
        }
    }
    for (i, (id, cat, dye)) in look.equips.iter().enumerate() {
        let rel = format!("Character/{cat}");
        let prefix = if *cat == "Weapon" { look.wprefix.clone() } else { String::new() };
        let Some(frame) = ctx.equip_frame_at(&rel, *id, cat, &prefix, action, idx, face) else { continue };
        match ctx.layers(&rel, *id, &frame, &[], look.skin) {
            Ok(mut l) if !l.is_empty() => {
                // 多種顏色的武器只畫第一種
                if l.iter().any(|x| x.name == "weapon1") {
                    l.retain(|x| !x.name.starts_with("weapon") || x.name == "weapon");
                }
                if let Some(d) = dye {
                    for layer in l.iter_mut() {
                        apply_dye(&mut layer.px, d);
                    }
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
    if !look.covered.is_empty() {
        let smap = &ctx.smap;
        parts.retain(|l| {
            // 頭髮依帽子種類處理（上面已經挑過），這裡只處理臉
            if l.owner != 1 {
                return true;
            }
            let codes = smap.get(&l.zname).map(String::as_str).unwrap_or("");
            !codes.as_bytes().chunks(2).any(|c| look.covered.iter().any(|v| v.as_bytes() == c))
        });
    }
    Ok((parts, delay, face))
}

/// 選單上的動作 → 遊戲裡的動作名稱（依武器挑選）
fn pick_action(ctx: &mut Ctx, look: &Look, want: &str) -> String {
    let broot = ctx.data.img("Character", &format!("{:08}.img", look.body_id)).ok();
    let wroot = look.equips.iter().find(|(_, c, _)| *c == "Weapon").and_then(|(wid, _, _)| ctx.data.img("Character/Weapon", &format!("{wid:08}.img")).ok());
    let body_has = |a: &str| broot.as_ref().map(|r| resolve(r, &format!("{a}/0"), 0).is_some()).unwrap_or(false);
    let weapon_has = |a: &str| wroot.as_ref().map(|r| resolve(r, &format!("{}{a}/0", look.wprefix), 0).is_some()).unwrap_or(true);
    let choose = |list: &[&str]| -> Option<String> {
        list.iter().find(|a| body_has(a) && weapon_has(a)).or_else(|| list.iter().find(|a| body_has(a))).map(|a| a.to_string())
    };
    match want {
        "attack" => {
            let prefs = attack_prefs(look.wtype);
            prefs
                .iter()
                .chain(ATTACK_ACTIONS.iter())
                .find(|a| body_has(a) && weapon_has(a))
                .map(|a| a.to_string())
                .or_else(|| choose(prefs))
                .unwrap_or_else(|| "swingO1".into())
        }
        "walk" => choose(if look.pose == "stand2" { &["walk2", "walk1"] } else { &["walk1", "walk2"] }).unwrap_or_else(|| "walk1".into()),
        "ladder" | "prone" | "sit" | "jump" => want.to_string(),
        _ => look.pose.to_string(),
    }
}

fn png_frame(c: &Composed, delay: i32) -> Result<AnimFrame, String> {
    let root = c.anchors.get("@root").copied().unwrap_or((0, 0));
    let anchors = ["brow", "neck", "navel"]
        .iter()
        .filter_map(|k| c.anchors.get(*k).map(|(x, y)| (k.to_string(), (x - root.0, y - root.1))))
        .collect();
    Ok(AnimFrame { png: png_data_url(c.w, c.h, &c.px)?, x: c.x0 - root.0, y: c.y0 - root.1, w: c.w, h: c.h, delay, anchors })
}

fn render_blocking(req: RenderReq) -> Result<RenderOut, String> {
    with_ctx(req.game_dir.as_deref(), |ctx| {
        let mut warnings = Vec::new();
        let head_only = req.mode.as_deref() == Some("head");
        let (look, equip_out) = resolve_look(ctx, &req, &mut warnings);
        let (hair_id, face_id) = (look.hair_id, look.face_id);

        // 只有頭（髮型、臉型的小圖）
        if head_only {
            let mut parts: Vec<Layer> = Vec::new();
            let mut head = ctx.layers("Character", look.head_id, "front", &["head", "humanEar"], look.skin).unwrap_or_default();
            if head.is_empty() {
                head = match ctx.layers("Character", look.head_id, "stand1/0", &["head"], look.skin) {
                    Ok(l) => l,
                    Err(e) => {
                        warnings.push(format!("頭：{e}"));
                        Vec::new()
                    }
                };
            }
            let has_head = !head.is_empty();
            parts.extend(head);
            if let Some(id) = look.face_id {
                match ctx.mixed("Character/Face", id, look.face_mix, "default", look.skin) {
                    Ok(l) => parts.extend(l),
                    Err(e) => warnings.push(format!("臉型：{e}")),
                }
            }
            if let Some(id) = look.hair_id {
                match ctx.mixed("Character/Hair", id, look.hair_mix, "default", look.skin) {
                    Ok(mut l) => {
                        // 長髮在身體後面的部分（hairBelowBody）也要畫，髮型才完整；default 沒有時用站姿的
                        if !l.iter().any(|x| x.name == "hairBelowBody") {
                            if let Ok(st) = ctx.mixed("Character/Hair", id, look.hair_mix, "stand1/0", look.skin) {
                                l.extend(st.into_iter().filter(|x| x.name == "hairBelowBody"));
                            }
                        }
                        let over = l.iter().any(|x| x.name == "hairOverHead");
                        parts.extend(l.into_iter().filter(|x| hair_visible(&x.name, CapType::None, over)));
                    }
                    Err(e) => warnings.push(format!("髮型：{e}")),
                }
            }
            let start: &[(&str, (i32, i32))] = if has_head { &[("neck", (0, 0))] } else { &[("brow", (0, 0))] };
            let (w, h, px, ax, ay) = compose(&parts, start).ok_or("沒有可以顯示的圖片")?;
            return Ok(RenderOut { png: png_data_url(w, h, &px)?, w, h, cx: ax, cy: ay, fx: None, fy: None, hair_id, face_id, warnings, equips: equip_out, anim: None });
        }

        // 全身：沒指定動作時只畫站姿第一格；指定動作時輸出每一格與特效
        let want = req.action.clone();
        let action = match &want {
            Some(w) => pick_action(ctx, &look, w),
            None => look.pose.to_string(),
        };
        let count = if want.is_some() && action != "jump" {
            let broot = ctx.data.img("Character", &format!("{:08}.img", look.body_id))?;
            (0..64).take_while(|i| resolve(&broot, &format!("{action}/{i}"), 0).is_some()).count().max(1)
        } else {
            1
        };
        // 普通攻擊的殘影；武器有自己的攻擊特效時不畫（和遊戲一樣只顯示武器的特效）
        let weapon_fx = want.as_deref() == Some("attack")
            && look.equips.iter().filter(|(_, c, _)| *c == "Weapon").map(|(id, _, _)| *id).collect::<Vec<_>>().into_iter().any(|id| ctx.has_attack_effect(id, &action));
        let after = if want.as_deref() == Some("attack") && !weapon_fx { ctx.afterimage(&look, &action) } else { HashMap::new() };
        let mut frames: Vec<AnimFrame> = Vec::new();
        let mut first: Option<Composed> = None;
        for i in 0..count {
            let (mut parts, delay, _) = match frame_parts(ctx, &look, &action, i, &mut warnings) {
                Ok(v) => v,
                Err(e) if i > 0 => {
                    warnings.push(e);
                    break;
                }
                Err(e) => return Err(e),
            };
            if let Some(extra) = after.get(&i) {
                parts.extend(extra.iter().cloned());
            }
            let c = compose_full(&parts, &[("navel", (0, 0))]).ok_or("沒有可以顯示的圖片")?;
            if want.is_some() {
                frames.push(png_frame(&c, delay)?);
            }
            if first.is_none() {
                first = Some(c);
            }
        }
        let c = first.ok_or("沒有可以顯示的圖片")?;
        let root = c.anchors.get("@root").copied().unwrap_or((0, 0));
        let navel = c.anchors.get("navel").copied().unwrap_or((0, 0));
        // 角色中心（肚臍往上一點），前端用來把角色擺在正中間，不受大型武器影響
        let (cx, cy) = (navel.0 - c.x0, navel.1 - c.y0 - 12);
        let (fx, fy) = (root.0 - c.x0, root.1 - c.y0);

        let anim = if want.is_some() {
            let mut effects = Vec::new();
            for &(id, _, dye) in &look.equips {
                if let Some(t) = ctx.item_effect(id, &action, dye) {
                    effects.push(t);
                }
            }
            // 站立時的中心（肚臍往上一點）相對於腳下，切換動作時角色不會跳動
            let center = ctx
                .layers("Character", look.body_id, &format!("{}/0", look.pose), &["body"], look.skin)
                .ok()
                .and_then(|l| l.into_iter().next())
                .and_then(|b| b.map.iter().find(|(k, _)| k == "navel").map(|(_, v)| *v))
                .map(|(x, y)| (x, y - 12))
                .unwrap_or((0, -40));
            Some(AnimOut { action: action.clone(), frames, effects, center })
        } else {
            None
        };
        let mut seen = std::collections::HashSet::new();
        warnings.retain(|w| seen.insert(w.clone()));
        Ok(RenderOut { png: png_data_url(c.w, c.h, &c.px)?, w: c.w, h: c.h, cx, cy, fx: Some(fx), fy: Some(fy), hair_id, face_id, warnings, equips: equip_out, anim })
    })
}

/* ---------------- 名牌戒指、聊天戒指 ---------------- */

/// UI 圖片的一個部分（名牌的左、中、右；對話框的九宮格與箭頭）
#[derive(Serialize, Clone)]
pub struct Piece {
    pub png: String,
    pub w: u32,
    pub h: u32,
    pub ox: i32,
    pub oy: i32,
}

/// 會動的名牌：一格動畫（delay 毫秒）
#[derive(Serialize, Clone)]
pub struct AniPiece {
    pub png: String,
    pub w: u32,
    pub h: u32,
    pub ox: i32,
    pub oy: i32,
    pub delay: i32,
}

#[derive(Serialize, Clone)]
pub struct UiSkin {
    pub id: i64,
    /// 文字顏色（#rrggbb）
    pub clr: Option<String>,
    pub pieces: HashMap<String, Piece>,
    /// 會動的名牌：「編號/ani/格數」或「編號/2（1、0）/格數」的動畫
    pub ani: Vec<AniPiece>,
    /// aniNameTag = 1：整個名牌都是動畫（不畫 c）
    pub ani_tag: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct TagsOut {
    pub name_tag: Option<UiSkin>,
    pub balloon: Option<UiSkin>,
    /// 讀不到時的說明（方便回報）
    pub notes: Vec<String>,
}

const NAME_TAG_KEYS: [&str; 3] = ["w", "c", "e"];
const BALLOON_KEYS: [&str; 11] = ["nw", "n", "ne", "w", "c", "e", "sw", "s", "se", "arrow", "head"];

/// 遊戲的顏色數值（有號 32 位元 ARGB）→ #rrggbb
fn argb_hex(v: i64) -> String {
    format!("#{:06x}", (v as u32) & 0x00FF_FFFF)
}

impl Ctx {
    /// 戒指 info：名牌編號（nameTag）、對話框編號（chatBalloon）、是否為點裝
    fn ring_info(&mut self, id: u32) -> (Option<i64>, Option<i64>, bool) {
        if let Some(v) = self.ring_info.get(&id) {
            return *v;
        }
        let v = self
            .data
            .img_child("Character/Ring", &format!("{id:08}.img"), "info")
            .ok()
            .flatten()
            .map(|info| {
                let int = |k: &str| info.get(k).and_then(|v| v.as_int());
                (int("nameTag"), int("chatBalloon"), int("cash") == Some(1))
            })
            .unwrap_or((None, None, false));
        self.ring_info.insert(id, v);
        v
    }

    /// 戒指的欄位：名牌戒指、聊天戒指各算一格（可以同時戴）；其他戒指看不出外觀
    fn ring_slot(&mut self, id: u32) -> &'static str {
        match self.ring_info(id) {
            (Some(_), _, _) => "RingTag",
            (None, Some(_), _) => "RingChat",
            _ => "Ring",
        }
    }

    fn ring_ids(&mut self, name: &str) -> Vec<u32> {
        if self.rings.is_none() {
            let list = self.data.img("String", "Eqp.img").map(|e| names_of(&e, "Ring")).unwrap_or_default();
            self.rings = Some(list);
        }
        let key = norm_name(name);
        self.rings.as_ref().map(|r| r.iter().filter(|(_, n)| norm_name(n) == key).map(|(id, _)| *id).collect()).unwrap_or_default()
    }

    /// UI/NameTag.img 或 UI/ChatBalloon.img 裡某個編號的圖片
    fn ui_skin(&mut self, img: &str, id: i64, keys: &[&str], notes: &mut Vec<String>) -> Option<UiSkin> {
        let Some(rel) = self.img_rel("UI", img) else {
            notes.push(format!("找不到 UI/{img}"));
            return None;
        };
        let root = self.data.img(&rel, img).ok()?;
        let base = id.to_string();
        let is_canvas = |p: &str| matches!(resolve(&root, p, 0), Some(Node::Canvas(..)));
        // 一般是「編號/c」；有動畫的是「編號/0/c」
        let path = [base.clone(), format!("{base}/0")]
            .into_iter()
            .find(|p| keys.iter().any(|k| is_canvas(&format!("{p}/{k}"))))
            .or_else(|| ["ani", "2", "1", "0"].iter().any(|d| is_canvas(&format!("{base}/{d}/0"))).then(|| base.clone()));
        let Some(path) = path else {
            let kids: Vec<String> = resolve(&root, &base, 0).map(|n| n.children().iter().map(|(k, _)| k.clone()).take(20).collect()).unwrap_or_default();
            notes.push(format!("UI/{img}/{id} 的內容：{}", if kids.is_empty() { "（沒有這個編號）".to_string() } else { kids.join("、") }));
            return None;
        };
        let clr = [format!("{path}/clr"), format!("{base}/clr")]
            .iter()
            .find_map(|p| resolve(&root, p, 0).and_then(|v| v.as_int()))
            .map(argb_hex);
        let mut pieces = HashMap::new();
        for k in keys {
            let Some(cn) = resolve(&root, &format!("{path}/{k}"), 0).cloned() else { continue };
            if !matches!(cn, Node::Canvas(..)) {
                continue;
            }
            let (ox, oy) = canvas_origin(&cn);
            if let Ok((w, h, px)) = self.data.pixels_of(&rel, img, &cn) {
                if let Ok(png) = png_data_url(w, h, &px) {
                    pieces.insert(k.to_string(), Piece { png, w, h, ox, oy });
                }
            }
        }
        // 會動的名牌（和 WzComparerR2 的讀法相同）：先找 ani，再找 2、1、0 底下的 0、1、2… 畫格
        let mut ani = Vec::new();
        let ani_tag = resolve(&root, &format!("{base}/aniNameTag"), 0).and_then(|v| v.as_int()).unwrap_or(0) != 0;
        if img == "NameTag.img" {
            let dir = ["ani", "2", "1", "0"].iter().map(|d| format!("{base}/{d}")).find(|d| is_canvas(&format!("{d}/0")));
            if let Some(dir) = dir {
                for f in 0..200 {
                    let Some(cn) = resolve(&root, &format!("{dir}/{f}"), 0).cloned() else { break };
                    if !matches!(cn, Node::Canvas(..)) {
                        break;
                    }
                    let (ox, oy) = canvas_origin(&cn);
                    let delay = canvas_delay(&cn).unwrap_or(100);
                    if let Ok((w, h, px)) = self.data.pixels_of(&rel, img, &cn) {
                        if let Ok(png) = png_data_url(w, h, &px) {
                            ani.push(AniPiece { png, w, h, ox, oy, delay });
                        }
                    }
                }
            }
        }
        if pieces.is_empty() && ani.is_empty() {
            return None;
        }
        Some(UiSkin { id, clr, pieces, ani, ani_tag })
    }
}

fn tags_of(ctx: &mut Ctx, rings: &[String], ids: &[u32]) -> TagsOut {
    let mut out = TagsOut::default();
    let (mut tag_id, mut chat_id) = (None, None);
    for &id in ids {
        let (t, c, _) = ctx.ring_info(id);
        if let Some(v) = t {
            tag_id.get_or_insert(v);
        }
        if let Some(v) = c {
            chat_id.get_or_insert(v);
        }
    }
    for name in rings {
        for id in ctx.ring_ids(name) {
            let Ok(Some(info)) = ctx.data.img_child("Character/Ring", &format!("{id:08}.img"), "info") else { continue };
            if let Some(v) = info.get("nameTag").and_then(|v| v.as_int()) {
                tag_id.get_or_insert(v);
            }
            if let Some(v) = info.get("chatBalloon").and_then(|v| v.as_int()) {
                chat_id.get_or_insert(v);
            }
            break;
        }
    }
    let mut notes = Vec::new();
    if let Some(id) = tag_id {
        out.name_tag = ctx.ui_skin("NameTag.img", id, &NAME_TAG_KEYS, &mut notes);
    }
    // 沒有聊天戒指時用一般的對話框（0）
    out.balloon = ctx.ui_skin("ChatBalloon.img", chat_id.unwrap_or(0), &BALLOON_KEYS, &mut notes);
    if out.balloon.is_none() && chat_id.is_some() {
        out.balloon = ctx.ui_skin("ChatBalloon.img", 0, &BALLOON_KEYS, &mut Vec::new());
    }
    out.notes = notes;
    out
}

/// 依戒指（名稱或編號）取得名牌與對話框的外觀
#[tauri::command]
pub async fn beauty_tags(game_dir: Option<String>, rings: Vec<String>, ids: Option<Vec<u32>>) -> Result<TagsOut, String> {
    let ids = ids.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || with_ctx(game_dir.as_deref(), |ctx| Ok(tags_of(ctx, &rings, &ids))))
        .await
        .map_err(|e| e.to_string())?
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
        // 戒指：只列出名牌戒指、聊天戒指
        if c == "Ring" {
            let (_, _, cash) = ctx.ring_info(id);
            let slot = ctx.ring_slot(id);
            if slot != "Ring" {
                list.push(ItemOut { id, name, slot, cash, dye: None });
            }
            continue;
        }
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

/// 裁掉圖示四周的透明邊（遊戲圖示的圖案常常偏在畫布一邊），再輸出 PNG
fn icon_png(w: u32, h: u32, px: &[u8]) -> Option<String> {
    if w <= 1 && h <= 1 {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) * 4 + 3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in y0..y1 {
        out.extend_from_slice(&px[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    png_data_url(cw as u32, ch as u32, &out).ok()
}

/* ---------------- 依名稱找道具圖示（黃金蘋果、閃亮彗星的獎池） ---------------- */

/// 名稱比對用：去掉空白，「祕」當成「秘」
fn norm_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '祕' { '秘' } else { c }).collect()
}

/// String 裡 Eqp 的分類 → Character 底下的資料夾
fn eqp_folder(cat: &str) -> Option<&'static str> {
    Some(match cat {
        "Cap" => "Character/Cap",
        "Cape" => "Character/Cape",
        "Coat" => "Character/Coat",
        "Glove" => "Character/Glove",
        "Longcoat" => "Character/Longcoat",
        "Pants" => "Character/Pants",
        "Shield" => "Character/Shield",
        "Shoes" => "Character/Shoes",
        "Weapon" => "Character/Weapon",
        "Ring" => "Character/Ring",
        "Accessory" => "Character/Accessory",
        "Android" => "Character/Android",
        "PetEquip" => "Character/PetEquip",
        "Taming" => "Character/TamingMob",
        "Dragon" => "Character/Dragon",
        "Mechanic" => "Character/Mechanic",
        "Bits" => "Character/Bits",
        "ArcaneForce" => "Character/ArcaneForce",
        "AuthenticForce" => "Character/AuthenticForce",
        "Jewel" => "Character/Jewel",
        _ => return None,
    })
}

fn build_item_names(ctx: &mut Ctx) -> HashMap<String, Vec<(&'static str, u32)>> {
    let mut map: HashMap<String, Vec<(&'static str, u32)>> = HashMap::new();
    let mut add = |name: &str, folder: &'static str, id: u32| {
        let k = norm_name(name);
        if !k.is_empty() {
            map.entry(k).or_default().push((folder, id));
        }
    };
    for (img, sub, folder) in [
        ("Consume.img", "", "Item/Consume"),
        ("Etc.img", "Etc", "Item/Etc"),
        ("Ins.img", "", "Item/Install"),
        ("Cash.img", "", "Item/Cash"),
        ("Pet.img", "", "Item/Pet"),
    ] {
        if let Ok(root) = ctx.data.img("String", img) {
            let node = if sub.is_empty() { Some(root.as_ref()) } else { root.get(sub) };
            for (id, item) in node.map(|n| n.children()).unwrap_or(&[]) {
                if let (Ok(id), Some(name)) = (id.parse::<u32>(), item.get("name").and_then(|v| v.as_str())) {
                    add(name, folder, id);
                }
            }
        }
    }
    if let Ok(eqp) = ctx.data.img("String", "Eqp.img") {
        if let Some(e) = eqp.get("Eqp") {
            for (cat, items) in e.children() {
                let Some(folder) = eqp_folder(cat) else { continue };
                for (id, item) in items.children() {
                    if let (Ok(id), Some(name)) = (id.parse::<u32>(), item.get("name").and_then(|v| v.as_str())) {
                        add(name, folder, id);
                    }
                }
            }
        }
    }
    map
}

impl Ctx {
    /// 某個道具的圖示：裝備在 Character/分類/編號.img/info/icon；
    /// 其他道具在 Item/種類/前四碼.img/編號/info/icon（寵物是一隻一個 img）
    fn item_icon(&mut self, folder: &str, id: u32) -> Option<String> {
        let node = if folder.starts_with("Character/") {
            let img = format!("{id:08}.img");
            let info = self.data.img_child(folder, &img, "info").ok()??;
            let icon = ["icon", "iconRaw"].iter().find_map(|k| info.get(k).filter(|n| matches!(n, Node::Canvas(..))))?.clone();
            let (w, h, px) = self.data.pixels_of(folder, &img, &icon).ok()?;
            return icon_png(w, h, &px);
        } else if folder == "Item/Pet" {
            let img = format!("{id}.img");
            let info = self.data.img_child(folder, &img, "info").ok()??;
            let icon = ["icon", "iconRaw"].iter().find_map(|k| info.get(k).filter(|n| matches!(n, Node::Canvas(..))))?.clone();
            (img, icon)
        } else {
            let img = format!("{:04}.img", id / 10000);
            let root = self.data.img(folder, &img).ok()?;
            let icon = ["info/icon", "info/iconRaw"]
                .iter()
                .find_map(|k| resolve(&root, &format!("{id:08}/{k}"), 0).filter(|n| matches!(n, Node::Canvas(..))))?
                .clone();
            (img, icon)
        };
        let (img, icon) = node;
        let (w, h, px) = self.data.pixels_of(folder, &img, &icon).ok()?;
        icon_png(w, h, &px)
    }

    /// 楓幣圖示：Item/Special/0900.img 的 09000000（銅幣）～09000003（楓幣袋）
    fn meso_icon(&mut self, id: u32) -> Option<String> {
        let folder = "Item/Special";
        let img = "0900.img";
        let root = self.data.img(folder, img).ok()?;
        let icon = ["iconRaw/0", "iconRaw", "icon/0", "icon", "info/iconRaw", "info/icon"]
            .iter()
            .find_map(|k| resolve(&root, &format!("{id:08}/{k}"), 0).filter(|n| matches!(n, Node::Canvas(..))))?
            .clone();
        let (w, h, px) = self.data.pixels_of(folder, img, &icon).ok()?;
        icon_png(w, h, &px)
    }

    /// 道具說明（String 裡的 desc）；裝備不讀
    fn item_desc(&mut self, folder: &str, id: u32) -> Option<String> {
        let (img, sub) = match folder {
            "Item/Consume" => ("Consume.img", ""),
            "Item/Etc" => ("Etc.img", "Etc"),
            "Item/Install" => ("Ins.img", ""),
            "Item/Cash" => ("Cash.img", ""),
            "Item/Pet" => ("Pet.img", ""),
            _ => return None,
        };
        let root = self.data.img("String", img).ok()?;
        let node = if sub.is_empty() { Some(root.as_ref()) } else { root.get(sub) }?;
        node.get(&id.to_string())?.get("desc")?.as_str().map(str::to_string)
    }

    /// 依名稱找道具圖示。名稱後面可以加「#提示」：同名的道具有好幾個時，
    /// 優先選說明裡有提示文字的（多個提示用 | 分開），例如「強烈的力量結晶#每週」
    fn icon_by_name(&mut self, full: &str) -> Option<String> {
        let full_key = norm_name(full);
        if let Some(v) = self.name_icons.get(&full_key) {
            return v.clone();
        }
        let (name, hint) = match full.split_once('#') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (full, None),
        };
        let key = norm_name(name);
        if hint.is_none() {
            if let Some(v) = self.name_icons.get(&key) {
                return v.clone();
            }
        }
        // 特殊名稱：@meso0～@meso3 是楓幣圖示
        if let Some(n) = key.strip_prefix("@meso").and_then(|n| n.parse::<u32>().ok()) {
            let found = self.meso_icon(9_000_000 + n.min(3));
            self.name_icons.insert(key, found.clone());
            return found;
        }
        if self.item_names.is_none() {
            let m = build_item_names(self);
            self.item_names = Some(m);
        }
        let mut cands = self.item_names.as_ref().and_then(|m| m.get(&key)).cloned().unwrap_or_default();
        if cands.is_empty() {
            // 名稱後面多了「(30天)」「100%」之類的說明時，去掉再找一次
            let short: String = key.split(['(', '（']).next().unwrap_or("").to_string();
            if short != key && !short.is_empty() {
                cands = self.item_names.as_ref().and_then(|m| m.get(&short)).cloned().unwrap_or_default();
            }
        }
        if let Some(h) = &hint {
            // 說明文字含提示字的排前面；「!字」表示含這個字的排最後（例如 !選擇 → 不要選擇型）
            let parts: Vec<String> = h.split('|').map(norm_name).filter(|x| !x.is_empty()).collect();
            let pos: Vec<&str> = parts.iter().filter(|x| !x.starts_with('!')).map(|x| x.as_str()).collect();
            let neg: Vec<&str> = parts.iter().filter_map(|x| x.strip_prefix('!')).filter(|x| !x.is_empty()).collect();
            let mut scored: Vec<(u8, (&'static str, u32))> = Vec::new();
            for c in cands {
                let d = self.item_desc(c.0, c.1).map(|d| norm_name(&d)).unwrap_or_default();
                let good = !pos.is_empty() && pos.iter().any(|x| d.contains(x));
                let bad = neg.iter().any(|x| d.contains(x));
                scored.push((if bad { 2 } else if good || pos.is_empty() { 0 } else { 1 }, c));
            }
            scored.sort_by_key(|(k, _)| *k);
            cands = scored.into_iter().map(|(_, c)| c).collect();
        }
        let mut found = None;
        for (folder, id) in cands.into_iter().take(12) {
            if let Some(png) = self.item_icon(folder, id) {
                found = Some(png);
                break;
            }
        }
        self.name_icons.insert(full_key, found.clone());
        found
    }
}

/// 依道具名稱取得圖示（PNG data URL），找不到的回傳 null
#[tauri::command]
pub async fn item_icons(game_dir: Option<String>, names: Vec<String>) -> Result<Vec<Option<String>>, String> {
    tauri::async_runtime::spawn_blocking(move || with_ctx(game_dir.as_deref(), |ctx| Ok(names.iter().map(|n| ctx.icon_by_name(n)).collect())))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone)]
pub struct StyleOut {
    pub id: u32,
    pub name: String,
    /// "M"（男）、"F"（女）、""（聯名等共用）
    pub g: &'static str,
}

/// 遊戲資料沒有髮型、臉型的性別欄位，照編號的千位數判斷（同名的男女版本幾乎都符合這個規則）：
/// 髮型 0、3、5、6 是男，1、4、7、8 是女；臉型 0、3、5、7 是男，1、4、6、8 是女；2、9 多半是聯名等共用
fn style_gender(hair: bool, id: u32) -> &'static str {
    let d = id / 1000 % 10;
    let (male, female): (&[u32], &[u32]) = if hair { (&[0, 3, 5, 6], &[1, 4, 7, 8]) } else { (&[0, 3, 5, 7], &[1, 4, 6, 8]) };
    if male.contains(&d) {
        "M"
    } else if female.contains(&d) {
        "F"
    } else {
        ""
    }
}

/// 遊戲資料裡所有的髮型或臉型（同一個造型的不同顏色只列一筆），新的在前面
#[tauri::command]
pub async fn beauty_styles(game_dir: Option<String>, kind: String) -> Result<Vec<StyleOut>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_ctx(game_dir.as_deref(), |ctx| {
            if let Some(v) = ctx.styles.get(&kind) {
                return Ok(v.clone());
            }
            let hair = kind == "hair";
            let rel = if hair { "Character/Hair" } else { "Character/Face" };
            let list = if hair { ctx.hair.clone() } else { ctx.face.clone() };
            let pkg = ctx.data.package(rel)?;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for (id, name) in list {
                let fam = if hair { id / 10 * 10 } else { id - (id / 100 % 10) * 100 };
                if seen.contains(&fam) || !pkg.imgs.contains_key(&format!("{id:08}.img")) {
                    continue;
                }
                seen.insert(fam);
                let name = if hair { strip_hair_color(&name).to_string() } else { name };
                out.push(StyleOut { id, name, g: style_gender(hair, id) });
            }
            out.sort_by(|a, b| b.id.cmp(&a.id));
            ctx.styles.insert(kind.clone(), out.clone());
            Ok(out)
        })
    })
    .await
    .map_err(|e| e.to_string())?
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

/* ---------------- 更新道具（itemup.rs 用） ---------------- */

/// 遊戲資料夾（找不到時為 None）
pub(crate) fn game_dir(hint: Option<&str>) -> Option<PathBuf> {
    find_game_dir(hint)
}

/// 遊戲檔有更新時丟掉之前讀好的遊戲資料，下次用到時重新讀（才不會讀到更新前的內容）
pub(crate) fn reset_ctx() {
    *CTX.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// 裝備資料夾（字串的分類對不到時，到這些資料夾找）
pub(crate) const EQUIP_DIRS: [&str; 20] = [
    "Character/Cap",
    "Character/Accessory",
    "Character/Coat",
    "Character/Longcoat",
    "Character/Pants",
    "Character/Shoes",
    "Character/Glove",
    "Character/Cape",
    "Character/Shield",
    "Character/Weapon",
    "Character/Ring",
    "Character/Android",
    "Character/PetEquip",
    "Character/TamingMob",
    "Character/Dragon",
    "Character/Mechanic",
    "Character/Bits",
    "Character/ArcaneForce",
    "Character/AuthenticForce",
    "Character/Jewel",
];

/// 不是裝備的道具放在哪個資料夾（照編號）：2 消耗、3 裝飾、4 其他、5 特殊（500 開頭是寵物，一隻一個 img）
fn item_folder_by_id(id: u32) -> Option<&'static str> {
    match id / 1_000_000 {
        2 => Some("Item/Consume"),
        3 => Some("Item/Install"),
        4 => Some("Item/Etc"),
        5 if id / 10_000 == 500 => Some("Item/Pet"),
        5 => Some("Item/Cash"),
        _ => None,
    }
}

/// 遊戲裡所有「有名稱、而且遊戲資料裡真的有」的道具：（編號, 資料夾, 名稱）與遊戲版本（String 的 WZ 版本，例如 282）。
/// 只有名稱、沒有資料的（例如還沒開放的）不算；髮型、臉型、膚色不算道具
pub(crate) fn all_items(hint: Option<&str>) -> Result<(u32, Vec<(u32, &'static str, String)>), String> {
    with_ctx(hint, |ctx| {
        let version = ctx.data.package("String")?.files.first().map(|f| f.version).unwrap_or(0);
        // 1. 字串裡的名稱（同一個編號在好幾個地方出現時只取第一個）
        let mut named: Vec<(u32, Option<&'static str>, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Ok(eqp) = ctx.data.img("String", "Eqp.img") {
            if let Some(e) = eqp.get("Eqp") {
                for (cat, items) in e.children() {
                    let Some(folder) = eqp_folder(cat) else { continue };
                    for (id, item) in items.children() {
                        if let (Ok(id), Some(name)) = (id.parse::<u32>(), item.get("name").and_then(|v| v.as_str())) {
                            if seen.insert(id) {
                                named.push((id, Some(folder), name.trim().to_string()));
                            }
                        }
                    }
                }
            }
        }
        for (img, sub) in [("Consume.img", ""), ("Etc.img", "Etc"), ("Ins.img", ""), ("Cash.img", ""), ("Pet.img", "")] {
            if let Ok(root) = ctx.data.img("String", img) {
                let node = if sub.is_empty() { Some(root.as_ref()) } else { root.get(sub) };
                for (id, item) in node.map(|n| n.children()).unwrap_or(&[]) {
                    if let (Ok(id), Some(name)) = (id.parse::<u32>(), item.get("name").and_then(|v| v.as_str())) {
                        if seen.insert(id) {
                            named.push((id, None, name.trim().to_string()));
                        }
                    }
                }
            }
        }
        // 2. 遊戲資料裡真的有的才算（裝備：分類資料夾裡有這個 img；其他：前四碼的 img 裡有這個編號）
        let dirs: Vec<&'static str> = EQUIP_DIRS.iter().copied().filter(|f| ctx.data.package(f).is_ok()).collect();
        let mut keys: HashMap<(&'static str, u32), std::collections::HashSet<u32>> = HashMap::new();
        let mut out = Vec::with_capacity(named.len());
        for (id, eq_folder, name) in named {
            let folder = if id / 1_000_000 == 1 {
                let img = format!("{id:08}.img");
                let mut found = None;
                for f in eq_folder.into_iter().chain(dirs.iter().copied()) {
                    if dirs.contains(&f) && ctx.data.package(f).map(|p| p.imgs.contains_key(&img)).unwrap_or(false) {
                        found = Some(f);
                        break;
                    }
                }
                match found {
                    Some(f) => f,
                    None => continue,
                }
            } else {
                let Some(f) = item_folder_by_id(id) else { continue };
                let exists = if f == "Item/Pet" {
                    ctx.data.package(f).map(|p| p.imgs.contains_key(&format!("{id}.img"))).unwrap_or(false)
                } else {
                    let prefix = id / 10_000;
                    let data = &mut ctx.data;
                    keys.entry((f, prefix))
                        .or_insert_with(|| {
                            data.img_keys(f, &format!("{prefix:04}.img")).map(|v| v.iter().filter_map(|k| k.parse().ok()).collect()).unwrap_or_default()
                        })
                        .contains(&id)
                };
                if !exists {
                    continue;
                }
                f
            };
            out.push((id, folder, name));
        }
        Ok((version, out))
    })
}

/// 這些裝備是不是點裝（info/cash = 1）；items：（編號, 資料夾）
pub(crate) fn cash_equips(hint: Option<&str>, items: &[(u32, String)]) -> Result<Vec<bool>, String> {
    with_ctx(hint, |ctx| {
        Ok(items
            .iter()
            .map(|(id, folder)| {
                folder.starts_with("Character/")
                    && ctx
                        .data
                        .img_child(folder, &format!("{id:08}.img"), "info")
                        .ok()
                        .flatten()
                        .and_then(|i| i.get("cash").and_then(|v| v.as_int()))
                        == Some(1)
            })
            .collect())
    })
}

/// 依編號取得道具圖示（items：[資料夾, 編號]），找不到的回傳 null
#[tauri::command]
pub async fn item_icons_by_id(game_dir: Option<String>, items: Vec<(String, u32)>) -> Result<Vec<Option<String>>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_ctx(game_dir.as_deref(), |ctx| Ok(items.iter().map(|(folder, id)| ctx.item_icon(folder, *id)).collect()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct ItemText {
    pub name: String,
    pub desc: String,
}

/// 道具在遊戲裡的名稱與說明
#[tauri::command]
pub async fn item_text(game_dir: Option<String>, folder: String, id: u32) -> Result<ItemText, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_ctx(game_dir.as_deref(), |ctx| {
            let key = id.to_string();
            let node: Option<Node> = if let Some(cat) = folder.strip_prefix("Character/") {
                // 騎寵的資料夾叫 TamingMob，字串裡叫 Taming
                let cat = if cat == "TamingMob" { "Taming" } else { cat };
                ctx.data.img("String", "Eqp.img").ok().and_then(|e| {
                    e.path(&format!("Eqp/{cat}/{key}")).cloned().or_else(|| {
                        // 字串的分類和資料所在的資料夾不一樣時，到其他分類找
                        e.get("Eqp").and_then(|eq| eq.children().iter().find_map(|(_, items)| items.get(&key).cloned()))
                    })
                })
            } else {
                let (img, sub) = match folder.as_str() {
                    "Item/Consume" => ("Consume.img", ""),
                    "Item/Etc" => ("Etc.img", "Etc"),
                    "Item/Install" => ("Ins.img", ""),
                    "Item/Cash" => ("Cash.img", ""),
                    "Item/Pet" => ("Pet.img", ""),
                    _ => ("", ""),
                };
                if img.is_empty() {
                    None
                } else {
                    ctx.data.img("String", img).ok().and_then(|r| {
                        let n = if sub.is_empty() { Some(r.as_ref()) } else { r.get(sub) };
                        n.and_then(|n| n.get(&key)).cloned()
                    })
                }
            };
            let n = node.ok_or("遊戲資料裡找不到這個道具")?;
            let text = |k: &str| n.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Ok(ItemText { name: text("name").trim().to_string(), desc: text("desc") })
        })
    })
    .await
    .map_err(|e| e.to_string())?
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
    fn style_genders() {
        assert_eq!(style_gender(true, 30000), "M");
        assert_eq!(style_gender(true, 31000), "F");
        assert_eq!(style_gender(true, 36670), "M");
        assert_eq!(style_gender(true, 48450), "F");
        assert_eq!(style_gender(true, 42280), "");
        assert_eq!(style_gender(false, 20000), "M");
        assert_eq!(style_gender(false, 21000), "F");
        assert_eq!(style_gender(false, 27000), "M");
        assert_eq!(style_gender(false, 26000), "F");
        assert_eq!(style_gender(false, 22035), "");
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
        // 純黑、純白不染；飽和度 -99 幾乎變灰
        let mut px = vec![0, 0, 0, 255, 255, 255, 255, 255, 204, 51, 51, 255];
        apply_dye(&mut px, &Dye { range: 0, hue: 0, sat: -99, val: 0 });
        assert_eq!(&px[0..3], &[0, 0, 0]);
        assert_eq!(&px[4..7], &[255, 255, 255]);
        assert!(px[8].abs_diff(px[9]) <= 17 && px[9].abs_diff(px[10]) <= 17);
        // 4 位元色的圖，結果仍是 17 的倍數
        assert!(px[8] % 17 == 0 && px[9] % 17 == 0 && px[10] % 17 == 0);
    }

    #[test]
    fn icon_trim() {
        // 4x2，只有右下角一個點不透明 → 裁成 1x1
        let mut px = vec![0u8; 4 * 2 * 4];
        px[(1 * 4 + 3) * 4 + 3] = 255;
        assert!(icon_png(4, 2, &px).is_some());
        assert!(icon_png(4, 2, &vec![0u8; 32]).is_none());
    }

    fn layer(name: &str, origin: (i32, i32), map: Vec<(&str, (i32, i32))>, w: u32, h: u32, z: i32) -> Layer {
        Layer {
            name: name.into(),
            zname: name.into(),
            owner: 0,
            z,
            origin,
            map: map.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            w,
            h,
            px: vec![255; (w * h * 4) as usize],
        }
    }

    #[test]
    fn compose_root_anchor() {
        // 身體原點 (16, 31)，肚臍在原點的 (-4, -20)；特效以腳下為準
        let body = layer("body", (16, 31), vec![("navel", (-4, -20)), ("neck", (-3, -30))], 32, 32, 10);
        let fx = layer("effect", (5, 5), vec![("@root", (0, 0))], 10, 10, i32::MIN / 2);
        let c = compose_full(&[body, fx], &[("navel", (0, 0))]).unwrap();
        let root = c.anchors["@root"];
        assert_eq!(root, (4, 20));
        assert_eq!(c.anchors["neck"], (1, -10));
        // 身體左上角 = 腳下 - 原點
        assert_eq!((c.x0, c.y0), (root.0 - 16, root.1 - 31));
        // 特效從 (root - 原點) 開始：x -1～9、y 15～25，身體 y -11～21
        assert_eq!(c.w, 32);
        assert_eq!(c.h, 36);
        let f = png_frame(&c, 100).unwrap();
        assert_eq!((f.x, f.y), (-16, -31));
        assert_eq!(f.anchors["navel"], (-4, -20));
    }

    #[test]
    fn merge_pieces() {
        let a = (2, 2, vec![255u8; 16], (1, 1));
        let b = (2, 2, vec![255u8; 16], (-3, 0));
        let (w, h, px, x, y) = merge_canvases(&[a, b]).unwrap();
        assert_eq!((w, h, x, y), (6, 3, -1, -1));
        assert_eq!(px.len(), 6 * 3 * 4);
        assert!(merge_canvases(&[]).is_none());
    }

    #[test]
    fn colors_hex() {
        assert_eq!(argb_hex(-1), "#ffffff");
        assert_eq!(argb_hex(-16777216), "#000000");
        assert_eq!(argb_hex(0xFF33_6699u32 as i32 as i64), "#336699");
    }

    #[test]
    fn hair_under_caps() {
        assert_eq!(cap_type(""), CapType::None);
        assert_eq!(cap_type("CpH5"), CapType::Headband);
        assert_eq!(cap_type("CpH1H5"), CapType::Half);
        assert_eq!(cap_type("CpH1H5H6"), CapType::Full);
        // 身體後面的長髮一律要畫
        for c in [CapType::None, CapType::Headband, CapType::Half, CapType::Full] {
            assert!(hair_visible("hairBelowBody", c, true));
        }
        assert!(hair_visible("hairOverHead", CapType::None, true));
        assert!(!hair_visible("hair", CapType::None, true));
        assert!(hair_visible("hair", CapType::None, false));
        assert!(hair_visible("hair", CapType::Half, true));
        assert!(!hair_visible("hairOverHead", CapType::Half, true));
        assert!(!hair_visible("hairShade", CapType::Full, true));
    }

    #[test]
    fn attack_by_weapon() {
        assert_eq!(attack_prefs(Some(40))[0], "swingT1");
        assert_eq!(attack_prefs(Some(45))[0], "shoot1");
        assert_eq!(attack_prefs(None)[0], "swingO1");
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
