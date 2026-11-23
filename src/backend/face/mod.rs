//! Bot faces as the official Hermes desktop draws them: the classic shapes
//! with eyes, the blobatar faces, and the colors, all as SVG, plus the
//! rasterizer that turns them into PNGs for the UI and for the server.
//!
//! Ported from the desktop's `plugins/hermes-bots/avatar.tsx` and
//! `lib/profile-color.ts`. Faces are drawn at rest; the desktop's idle
//! animation (blink, sway) is not reproduced.

pub mod blobatar;

use std::f64::consts::PI;

use serde_json::Value;

pub use blobatar::Kind as BlobKind;

/// The friendly violet the primary profile wears.
pub const PRIMARY_COLOR: &str = "#8b5cf6";

/// The flat shapes a bot can be seeded with.
const AVATAR_SHAPES: [&str; 7] = ["circle", "squircle", "pill", "triangle", "hexagon", "cloud", "drop"];

/// The classic shapes the picker offers, after the blobatar tile.
pub const PICKER_SHAPES: [&str; 8] = ["circle", "blob", "squircle", "pill", "triangle", "hexagon", "cloud", "drop"];

const SATURATION: u32 = 68;
const LIGHTNESS: u32 = 58;

/// The twelve picker swatches: evenly spaced hues at the palette's tone.
pub fn swatches() -> Vec<String> {
    (0..12).map(|i| format!("hsl({} {SATURATION}% {LIGHTNESS}%)", i * 30)).collect()
}

/// `hash * 31 + unit` over UTF-16 units, as `profile-color.ts` hashes.
fn hash_units(units: impl Iterator<Item = u16>) -> u32 {
    units.fold(0u32, |h, u| h.wrapping_mul(31).wrapping_add(u as u32))
}

/// The deterministic hue for a named profile; none for the primary.
pub fn profile_color(name: &str) -> Option<String> {
    let key = name.trim();
    if key.is_empty() || key == "default" {
        return None;
    }
    Some(format!("hsl({} {SATURATION}% {LIGHTNESS}%)", hash_units(key.encode_utf16()) % 360))
}

/// A picked color wins; with none, the name's hue, else the primary violet.
pub fn avatar_color(color: Option<&str>, name: &str) -> String {
    color
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .or_else(|| profile_color(name))
        .unwrap_or_else(|| PRIMARY_COLOR.into())
}

/// The shape a bot gets before anyone picks one.
pub fn default_shape_for(name: &str) -> &'static str {
    // `for (const ch of name)` + `charCodeAt(0)`: the first UTF-16 unit of
    // each code point.
    let hash = hash_units(name.chars().map(|c| {
        let mut buf = [0u16; 2];
        c.encode_utf16(&mut buf)[0]
    }));
    AVATAR_SHAPES[hash as usize % AVATAR_SHAPES.len()]
}

/// A bot's look: what the editor opens with and what the face draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub shape: String,
    /// `None` = no explicit pick, i.e. the name's hue.
    pub color: Option<String>,
}

/// `botAppearance`: the primary profile is a violet squircle until someone
/// customizes it; every other bot follows its metadata, else its name.
pub fn appearance(name: &str, meta: &Value) -> Appearance {
    let text = |k: &str| meta.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let custom = meta.get("custom").and_then(Value::as_bool).unwrap_or(false);
    if name.trim().eq_ignore_ascii_case("default") && !custom {
        return Appearance { shape: "squircle".into(), color: Some(PRIMARY_COLOR.into()) };
    }
    Appearance {
        shape: text("shape").unwrap_or_else(|| default_shape_for(name).into()),
        color: text("color").or_else(|| profile_color(name)),
    }
}

// ── blobatar shape strings ──────────────────────────────────────────────────
//   'blobatar'               the face follows the bot's name
//   'blobatar:<seed>'        seed locked
//   'blobatar:<seed>:<kind>' plus a silhouette pinned
//   'blobatar::<kind>'       silhouette pinned, seed follows the name

pub fn is_blob_shape(shape: &str) -> bool {
    shape == "blobatar" || shape.starts_with("blobatar:")
}

/// (locked seed or "", pinned silhouette).
pub fn parse_blob_shape(shape: &str) -> (String, Option<BlobKind>) {
    let mut parts = shape.split(':').skip(1);
    let seed = parts.next().unwrap_or_default().to_string();
    let kind = parts.next().and_then(BlobKind::parse);
    (seed, kind)
}

pub fn blob_shape_string(seed: &str, kind: Option<BlobKind>) -> String {
    match kind {
        Some(k) => format!("blobatar:{seed}:{}", k.name()),
        None if seed.is_empty() => "blobatar".into(),
        None => format!("blobatar:{seed}"),
    }
}

// ── colors for the rasterizer ───────────────────────────────────────────────

/// `#rrggbb` for a CSS color the desktop stores: hex or `hsl()`, with spaces
/// or commas. Anything else passes through for the SVG parser to try.
pub fn css_to_hex(color: &str) -> String {
    let c = color.trim();
    let Some(inner) = c.strip_prefix("hsl(").or_else(|| c.strip_prefix("hsla(")).and_then(|s| s.strip_suffix(')')) else {
        return c.to_string();
    };
    let nums: Vec<f64> = inner
        .split(|ch: char| ch == ',' || ch == '/' || ch.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.trim_end_matches('%').trim_end_matches("deg").parse().ok())
        .collect();
    let [h, s, l] = [nums.first(), nums.get(1), nums.get(2)].map(|v| v.copied().unwrap_or(0.0));
    let (s, l) = (s / 100.0, l / 100.0);
    let a = s * l.min(1.0 - l);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", byte(f(0.0)), byte(f(8.0)), byte(f(4.0)))
}

/// The desktop's `isDarkColor`, quirks included: it reads the color as hex
/// with `parseInt`, so an `hsl()` color parses as NaN and counts as dark.
fn is_dark_color(color: &str) -> bool {
    let digits: String = color.chars().skip(1).take_while(|c| c.is_ascii_hexdigit()).collect();
    let n = u64::from_str_radix(&digits, 16).unwrap_or(0) as u32 as i32;
    let (r, g, b) = ((n >> 16) & 255, (n >> 8) & 255, n & 255);
    (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) < 110.0
}

// ── the classic math face ───────────────────────────────────────────────────

type Point = (f64, f64);

fn cubic_at(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    (
        u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0,
        u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1,
    )
}

fn drop_ring(steps: usize) -> Vec<Point> {
    let n = (steps / 3).max(8);
    let mut pts = Vec::new();
    for i in 0..n {
        pts.push(cubic_at((20.0, 3.0), (20.0, 3.0), (6.0, 20.0), (6.0, 27.0), i as f64 / n as f64));
    }
    for i in 0..=n {
        let t = (i as f64 / n as f64) * PI;
        pts.push((20.0 - 14.0 * t.cos(), 27.0 + 13.5 * t.sin()));
    }
    for i in 1..=n {
        pts.push(cubic_at((34.0, 27.0), (34.0, 20.0), (20.0, 3.0), (20.0, 3.0), i as f64 / n as f64));
    }
    pts
}

/// The outline of a face in a 40×40 box, sampled from formulas.
fn face_ring(shape: &str, steps: usize) -> Vec<Point> {
    let kind = if shape.starts_with("sigil-") { "circle" } else { shape };
    if kind == "drop" || kind == "teardrop" {
        return drop_ring(steps);
    }
    (0..steps)
        .map(|i| {
            let a = (i as f64 / steps as f64) * PI * 2.0 - PI / 2.0;
            let (c, s) = (a.cos(), a.sin());
            let (rx, ry) = match kind {
                "blob" => {
                    let r = 16.0 + 1.7 * (3.0 * a).sin() + 0.7 * (5.0 * a).cos();
                    (r, r)
                }
                "squircle" => {
                    let d = (c.abs().powi(5) + s.abs().powi(5)).powf(1.0 / 5.0);
                    let r = 16.2 / if d == 0.0 { 1.0 } else { d };
                    (r, r)
                }
                "pill" => {
                    let d = (c.abs().powi(8) + (s / 0.72).abs().powi(8)).powf(1.0 / 8.0);
                    let r = 16.0 / if d == 0.0 { 1.0 } else { d };
                    (r, r)
                }
                "triangle" | "tetrahedron" | "wedge" => {
                    let u = (a + PI / 2.0 + PI * 2.0) % (PI * 2.0);
                    let sector = (u / ((PI * 2.0) / 3.0)) % 1.0;
                    let r = 13.5 / ((sector - 0.5) * 1.9).cos().max(0.42);
                    (r, r)
                }
                "hexagon" | "hex" | "icosahedron" | "dodecahedron" => {
                    let seg = PI / 3.0;
                    let r = 16.2 * (seg / 2.0).cos() / (a - seg * (a / seg).round()).cos();
                    (r, r)
                }
                "cube" | "octahedron" => {
                    let d = (c.abs().powf(3.1) + s.abs().powf(3.1)).powf(1.0 / 3.1);
                    let r = 16.0 / if d == 0.0 { 1.0 } else { d };
                    (r, r)
                }
                "pebble" => (16.4 * (1.04 - 0.14 * (2.0 * a).cos()), 15.2 * (1.06 + 0.08 * (2.0 * a).sin())),
                _ => (16.2, 16.2),
            };
            (20.0 + rx * c, 20.0 + ry * s)
        })
        .collect()
}

fn ring_path(pts: &[Point]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        d += &format!("{}{x:.2} {y:.2}", if i == 0 { "M" } else { "L" });
    }
    d + "Z"
}

const CLOUD_PATH: &str = "M11 32 a7.5 7.5 0 0 1 -1 -14.9 A9.5 9.5 0 0 1 29 12.5 A7 7 0 0 1 30 32 Z";

/// The classic face: a filled outline with two eyes and catchlights.
fn math_face(shape: &str, color: &str, size: u32) -> String {
    let fill = css_to_hex(color);
    let dark = is_dark_color(color);
    let eye = if dark { "rgba(232,220,195,0.95)" } else { "rgba(0,0,0,0.85)" };
    let hl = if dark { "rgba(0,0,0,0.6)" } else { "rgba(255,255,255,0.85)" };
    let body = if shape == "cloud" { CLOUD_PATH.to_string() } else { ring_path(&face_ring(shape, 52)) };
    let y = if shape == "cloud" { 22.0 } else { 17.2 };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 40 44\">\
         <path d=\"{body}\" fill=\"{fill}\"/>\
         <ellipse cx=\"15.4\" cy=\"{y}\" rx=\"2.2\" ry=\"2.3\" fill=\"{eye}\"/>\
         <ellipse cx=\"24.6\" cy=\"{y}\" rx=\"2.2\" ry=\"2.3\" fill=\"{eye}\"/>\
         <circle cx=\"14.8\" cy=\"{}\" r=\"0.65\" fill=\"{hl}\"/>\
         <circle cx=\"24\" cy=\"{}\" r=\"0.65\" fill=\"{hl}\"/></svg>",
        y - 0.7,
        y - 0.7
    )
}

// ── sigils (render-only, for bots that picked one in older desktops) ───────

fn sigil_rng(text: &str) -> impl FnMut() -> f64 + use<> {
    let mut h: u32 = 2166136261;
    for u in text.encode_utf16() {
        h ^= u as u32;
        h = h.wrapping_mul(16777619);
    }
    let mut state = if h == 0 { 88675123 } else { h };
    move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f64 / 4294967296.0
    }
}

fn sigil_face(shape: &str, color: &str, name: &str, size: u32) -> String {
    let seed: i64 = shape[6..].parse().unwrap_or(0);
    let mut rng = sigil_rng(&format!("{name}::{seed}"));
    let gx = |i: i64| 6 + i * 7;
    let gy = |j: i64| 8 + j * 6;
    let mut strokes = Vec::new();
    let segments = 4 + (rng() * 3.0).floor() as i64;
    for _ in 0..segments {
        let x1 = (rng() * 3.0).floor() as i64;
        let y1 = (rng() * 5.0).floor() as i64;
        let x2 = (x1 + if rng() > 0.5 { 1 } else { -1 }).clamp(0, 2);
        let y2 = (y1 + (rng() * 3.0).floor() as i64 - 1).clamp(0, 4);
        strokes.push(format!("M{} {} L{} {}", gx(x1), gy(y1), gx(x2), gy(y2)));
        strokes.push(format!("M{} {} L{} {}", gx(4 - x1), gy(y1), gx(4 - x2), gy(y2)));
        if rng() > 0.6 {
            strokes.push(format!("M{} {} L{} {}", gx(x2), gy(y2), gx(4 - x2), gy(y2)));
        }
    }
    strokes.push(format!("M20 {} L20 {}", gy(0), gy(4)));
    let fill = css_to_hex(color);
    let ring = if rng() > 0.45 {
        format!("<path d=\"M20 4 L36 20 L20 36 L4 20 Z\" fill=\"none\" opacity=\"0.5\" stroke=\"{fill}\" stroke-width=\"1.2\"/>")
    } else {
        String::new()
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 40 40\">{ring}\
         <path d=\"{}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"2.2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\
         <circle cx=\"16\" cy=\"14\" r=\"2.4\" fill=\"{fill}\"/><circle cx=\"24\" cy=\"14\" r=\"2.4\" fill=\"{fill}\"/></svg>",
        strokes.join(" ")
    )
}

/// The face for any shape string the desktop stores, as SVG.
/// `name` is the profile name, which seeds name-following blobatars.
pub fn svg(shape: &str, color: &str, name: &str, size: u32) -> String {
    if is_blob_shape(shape) {
        let (seed, kind) = parse_blob_shape(shape);
        let seed = if !seed.is_empty() { seed } else if !name.is_empty() { name.to_string() } else { "agent".into() };
        return blobatar::svg(&seed, kind, size);
    }
    if shape.starts_with("sigil-") {
        return sigil_face(shape, color, name, size);
    }
    math_face(if shape.is_empty() { "circle" } else { shape }, color, size)
}

/// Renders SVG markup to a `px`×`px` PNG.
pub fn rasterize(svg: &str, px: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = (px as f32 / size.width()).min(px as f32 / size.height());
    let dx = (px as f32 - size.width() * scale) / 2.0;
    let dy = (px as f32 - size.height() * scale) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

/// An uploaded picture as the desktop stores one: the centered square,
/// scaled to `edge`×`edge`, as PNG.
pub fn normalize_image(bytes: &[u8], edge: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let side = img.width().min(img.height());
    let square = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side);
    let scaled = square.resize_exact(edge, edge, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    scaled.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn colors_follow_the_desktop() {
        assert_eq!(profile_color("default"), None);
        assert_eq!(profile_color("work").unwrap(), format!("hsl({} 68% 58%)", hash_units("work".encode_utf16()) % 360));
        assert_eq!(avatar_color(None, "default"), PRIMARY_COLOR);
        assert_eq!(avatar_color(Some("#123456"), "work"), "#123456");
        assert_eq!(swatches()[1], "hsl(30 68% 58%)");
        assert_eq!(css_to_hex("hsl(0 100% 50%)"), "#ff0000");
        assert_eq!(css_to_hex("hsl(120, 100%, 25%)"), "#008000");
        assert_eq!(css_to_hex("#abcdef"), "#abcdef");
        // The desktop's quirk: hsl() colors count as dark, so eyes go cream.
        assert!(is_dark_color("hsl(60 68% 58%)"));
        assert!(!is_dark_color(PRIMARY_COLOR));
        assert!(is_dark_color("#222222"));
    }

    #[test]
    fn appearance_defaults() {
        assert_eq!(appearance("default", &json!({})), Appearance { shape: "squircle".into(), color: Some(PRIMARY_COLOR.into()) });
        let custom = appearance("default", &json!({ "custom": true, "shape": "pill" }));
        assert_eq!(custom.shape, "pill");
        assert_eq!(custom.color, None);
        let work = appearance("work", &json!({}));
        assert_eq!(work.shape, default_shape_for("work"));
        assert_eq!(work.color, profile_color("work"));
    }

    #[test]
    fn blob_shape_strings_round_trip() {
        for (shape, seed, kind) in [
            ("blobatar", "", None),
            ("blobatar:abc", "abc", None),
            ("blobatar:abc:sun", "abc", Some(BlobKind::Sun)),
            ("blobatar::boxy", "", Some(BlobKind::Boxy)),
        ] {
            assert!(is_blob_shape(shape));
            assert_eq!(parse_blob_shape(shape), (seed.to_string(), kind));
            assert_eq!(blob_shape_string(seed, kind), shape);
        }
        assert!(!is_blob_shape("circle"));
    }

    #[test]
    fn every_shape_rasterizes() {
        let shapes = PICKER_SHAPES.iter().copied().chain(["blobatar", "blobatar::sun", "sigil-3", "cube", "pebble"]);
        for shape in shapes {
            let png = rasterize(&svg(shape, "hsl(200 68% 58%)", "work", 40), 160).unwrap_or_else(|| panic!("{shape}"));
            let img = image::load_from_memory(&png).unwrap();
            assert_eq!((img.width(), img.height()), (160, 160));
            // Something was drawn in the middle of the frame.
            assert!(img.to_rgba8().get_pixel(80, 80)[3] > 0, "{shape} is empty");
        }
    }

    #[test]
    fn uploads_become_centered_squares() {
        let wide = image::RgbaImage::from_pixel(300, 100, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(wide).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let png = normalize_image(&bytes.into_inner(), 256).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (256, 256));
    }
}

/// Writes every face to PNGs for a look:
/// `HERMES_BOT_FACES=/some/dir cargo test face_sheet -- --ignored`.
#[cfg(test)]
mod sheet {
    #[test]
    #[ignore]
    fn face_sheet() {
        let Ok(dir) = std::env::var("HERMES_BOT_FACES") else { return };
        let mut shapes: Vec<String> = super::PICKER_SHAPES.iter().map(|s| s.to_string()).collect();
        shapes.push("blobatar".into());
        shapes.extend(super::BlobKind::ALL.iter().map(|k| super::blob_shape_string("", Some(*k))));
        shapes.push("sigil-2".into());
        for (i, shape) in shapes.iter().enumerate() {
            for (j, color) in ["#8b5cf6", "hsl(330 68% 58%)", "hsl(120 68% 58%)"].iter().enumerate() {
                let png = super::rasterize(&super::svg(shape, color, "homelab", 40), 160).unwrap();
                std::fs::write(format!("{dir}/{i:02}-{j}.png"), png).unwrap();
            }
        }
    }
}
