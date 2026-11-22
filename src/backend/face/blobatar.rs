//! A Rust port of the static renderer of blobatar 2.7.0
//! (<https://www.npmjs.com/package/blobatar>, MIT, © 2026 Alain), the
//! library the official Hermes desktop draws its "blobatar" bot faces with.
//!
//! Only what `blobatar/blob`'s `blobatar(seed, { size, traits })` needs is
//! here: seed hashing, traits, the OKLCh palette, the ten silhouettes and the
//! SVG serialization. The output is byte-identical to the library's, which
//! the golden test in this file checks against SVGs produced by the real one.

use std::f64::consts::PI;

use unicode_normalization::UnicodeNormalization;

// ── numbers, the way JavaScript prints them ─────────────────────────────────

/// `Math.round`: halves go toward +∞.
fn js_round(v: f64) -> f64 {
    let f = v.floor();
    if v - f >= 0.5 { f + 1.0 } else { f }
}

/// `String(n)` for the finite, short values the library emits.
fn js_num(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    format!("{v}")
}

/// The library's `r2`: two decimals, and never "-0".
fn r2(v: f64) -> String {
    js_num(js_round(v * 100.0) / 100.0)
}

/// `r2` that stays a number, as `compose.ts` uses it for circles.
fn r2n(v: f64) -> f64 {
    js_round(v * 100.0) / 100.0
}

// ── hash.ts ─────────────────────────────────────────────────────────────────

fn feed(mut h: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        h = (h ^ b as u32).wrapping_mul(3432918353);
        h = h.rotate_left(13);
    }
    h
}

fn finalize(mut h: u32) -> u32 {
    h = (h ^ (h >> 16)).wrapping_mul(2246822507);
    h = (h ^ (h >> 13)).wrapping_mul(3266489909);
    h ^ (h >> 16)
}

fn seed_state(seed: &str) -> u32 {
    let s: String = seed.nfc().collect::<String>().trim().to_lowercase();
    // JavaScript's `length` counts UTF-16 units.
    let len = s.encode_utf16().count() as u32;
    feed(1779033703 ^ len, s.as_bytes())
}

fn stream(state: u32, key: &str) -> f64 {
    finalize(feed(feed(state, &[0xff]), key.as_bytes())) as f64 / 4294967296.0
}

// ── traits.ts ───────────────────────────────────────────────────────────────

struct Traits {
    state: u32,
    /// The pinned `shape` trait, when a silhouette is chosen.
    shape: Option<f64>,
}

impl Traits {
    fn get(&self, key: &str) -> f64 {
        match (key, self.shape) {
            ("shape", Some(o)) => {
                if o > 0.0 {
                    if o < 1.0 { o } else { 0.999999 }
                } else {
                    0.0
                }
            }
            _ => stream(self.state, key),
        }
    }
    fn num(&self, key: &str, min: f64, max: f64) -> f64 {
        min + self.get(key) * (max - min)
    }
    fn int(&self, key: &str, min: i64, max: i64) -> i64 {
        min + (self.get(key) * (max - min + 1) as f64).floor() as i64
    }
    fn jitter(&self, key: &str, amount: f64) -> f64 {
        (self.get(key) * 2.0 - 1.0) * amount
    }
}

// ── color.ts ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Oklch {
    l: f64,
    c: f64,
    h: f64,
}

fn to_linear(Oklch { l, c, h }: Oklch) -> [f64; 3] {
    let r = (h * PI) / 180.0;
    let a = c * r.cos();
    let b = c * r.sin();
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.291485548 * b;
    let (bl, bm, bs) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.0767416621 * bl - 3.3077115913 * bm + 0.2309699292 * bs,
        -1.2684380046 * bl + 2.6097574011 * bm - 0.3413193965 * bs,
        -0.0041960863 * bl - 0.7034186147 * bm + 1.707614701 * bs,
    ]
}

fn in_gamut(rgb: &[f64; 3]) -> bool {
    rgb.iter().all(|&v| v >= -1e-4 && v <= 1.0 + 1e-4)
}

fn resolve(color: Oklch) -> [f64; 3] {
    let mut rgb = to_linear(color);
    if !in_gamut(&rgb) {
        let (mut lo, mut hi) = (0.0, color.c);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(&to_linear(Oklch { c: mid, ..color })) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = to_linear(Oklch { c: lo, ..color });
    }
    rgb.map(|v| v.clamp(0.0, 1.0))
}

fn luminance(color: Oklch) -> f64 {
    let [r, g, b] = resolve(color);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: Oklch, b: Oklch) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn ensure_contrast(fg: Oklch, bg: Oklch, min: f64) -> Oklch {
    if contrast(fg, bg) >= min {
        return fg;
    }
    let lean = if fg.l >= bg.l { 1.0 } else { -1.0 };
    for dir in [lean, -lean] {
        let mut probe = fg;
        for _ in 0..60 {
            probe.l = (probe.l + dir * 0.02).clamp(0.0, 1.0);
            if contrast(probe, bg) >= min {
                return probe;
            }
            if probe.l == 0.0 || probe.l == 1.0 {
                break;
            }
        }
    }
    let black = Oklch { l: 0.0, c: 0.0, ..fg };
    let white = Oklch { l: 1.0, c: 0.0, ..fg };
    if contrast(black, bg) >= contrast(white, bg) { black } else { white }
}

fn to_hex(color: Oklch) -> String {
    let mut out = String::from("#");
    for v in resolve(color) {
        let s = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        out.push_str(&format!("{:02x}", js_round(s * 255.0) as i64));
    }
    out
}

const TONES: [(f64, f64, f64); 6] = [
    (0.2, 0.86, 0.085),
    (0.36, 0.9, 0.028),
    (0.62, 0.73, 0.135),
    (0.8, 0.62, 0.165),
    (0.93, 0.87, 0.16),
    (1.0, 0.34, 0.035),
];

const DARK_SURFACE: Oklch = Oklch { l: 0.145, c: 0.0, h: 0.0 };

/// The figure's two colors: (head, eye).
fn palette(hue: f64, tone: f64) -> (String, String) {
    let (_, l, c) = TONES.iter().copied().find(|(edge, _, _)| tone < *edge).unwrap_or(TONES[0]);
    let mut head = ensure_contrast(Oklch { l, c, h: hue }, DARK_SURFACE, 1.5);
    let bg = Oklch { l: 0.965, c: 0.01, h: hue };
    let mut eye = if head.l >= 0.5 { Oklch { l: 0.17, c: 0.02, h: hue } } else { Oklch { l: 0.97, c: 0.012, h: hue } };
    head = ensure_contrast(head, bg, 1.25);
    eye = ensure_contrast(eye, head, 4.5);
    (to_hex(head), to_hex(eye))
}

// ── shape.ts ────────────────────────────────────────────────────────────────

fn superellipse(cx: f64, cy: f64, rx: f64, ry: f64, n: f64, rot: f64) -> String {
    let k = ((8.0 * 2f64.powf(-1.0 / n) - 4.0) / 3.0).min(1.0);
    let (a, b) = (rx, ry);
    let (ak, bk) = (a * k, b * k);
    let pts = [
        (a, 0.0),
        (a, bk), (ak, b), (0.0, b),
        (-ak, b), (-a, bk), (-a, 0.0),
        (-a, -bk), (-ak, -b), (0.0, -b),
        (ak, -b), (a, -bk), (a, 0.0),
    ];
    let t = (rot * PI) / 180.0;
    let (cos, sin) = (t.cos(), t.sin());
    let at = |i: usize| {
        let (x, y) = pts[i];
        format!("{} {}", r2(cx + x * cos - y * sin), r2(cy + x * sin + y * cos))
    };
    let mut d = format!("M{}", at(0));
    let mut i = 1;
    while i < 13 {
        d += &format!("C{} {} {}", at(i), at(i + 1), at(i + 2));
        i += 3;
    }
    d + "Z"
}

fn blob_path(cx: f64, cy: f64, rx: f64, ry: f64, radii: &[f64], rot: f64) -> String {
    let n = radii.len() as i64;
    let t0 = (rot * PI) / 180.0;
    let p: Vec<(f64, f64)> = radii
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let a = t0 + (2.0 * PI * i as f64) / n as f64;
            (cx + rx * m * a.cos(), cy + ry * m * a.sin())
        })
        .collect();
    let at = |i: i64| p[(((i % n) + n) % n) as usize];
    let mut d = format!("M{} {}", r2(at(0).0), r2(at(0).1));
    for i in 0..n {
        let (x0, y0) = at(i - 1);
        let (x1, y1) = at(i);
        let (x2, y2) = at(i + 1);
        let (x3, y3) = at(i + 2);
        d += &format!(
            "C{} {} {} {} {} {}",
            r2(x1 + (x2 - x0) / 6.0),
            r2(y1 + (y2 - y0) / 6.0),
            r2(x2 - (x3 - x1) / 6.0),
            r2(y2 - (y3 - y1) / 6.0),
            r2(x2),
            r2(y2)
        );
    }
    d + "Z"
}

fn polygon(cx: f64, cy: f64, rx: f64, ry: f64, sides: i64, round: f64, rot: f64) -> String {
    let k = if round > 0.0 { if round < 1.0 { round / 2.0 } else { 0.5 } } else { 0.0 };
    let t0 = (rot * PI) / 180.0 - PI / 2.0;
    let v: Vec<(f64, f64)> = (0..sides)
        .map(|i| {
            let a = t0 + (2.0 * PI * i as f64) / sides as f64;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect();
    let at = |i: i64| v[(((i % sides) + sides) % sides) as usize];
    let cut = |i: i64, j: i64| {
        let (x0, y0) = at(i);
        let (x1, y1) = at(j);
        format!("{} {}", r2(x0 + (x1 - x0) * k), r2(y0 + (y1 - y0) * k))
    };
    let mut d = format!("M{}", cut(0, -1));
    for i in 0..sides {
        let (x, y) = at(i);
        d += &format!("Q{} {} {}", r2(x), r2(y), cut(i, i + 1));
        if k < 0.5 {
            d += &format!("L{}", cut(i + 1, i));
        }
    }
    d + "Z"
}

fn box_path(cx: f64, cy: f64, rx: f64, ry: f64) -> String {
    let (l, r) = (r2(cx - rx), r2(cx + rx));
    format!("M{l} {}H{r}V{}H{l}Z", r2(cy - ry), r2(cy + ry))
}

fn taper(cx: f64, cy: f64, rx: f64, ry: f64, tip: f64) -> String {
    let t = tip.max(1.05);
    let tx = rx * (1.0 - 1.0 / (t * t)).sqrt();
    let ty = cy - ry / t;
    let apex = cy - t * ry;
    let px = tx * 0.14;
    let py = ty + 0.86 * (apex - ty);
    format!(
        "M{} {}L{} {}Q{} {} {} {}L{} {}Z",
        r2(cx - tx),
        r2(ty),
        r2(cx - px),
        r2(py),
        r2(cx),
        r2(apex),
        r2(cx + px),
        r2(py),
        r2(cx + tx),
        r2(ty)
    )
}

// ── styles/shapes.ts + compose.ts ───────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Round,
    Organic,
    Boxy,
    Capsule,
    Nub,
    Cloud,
    Droplet,
    Hexagon,
    Sun,
    Triangle,
}

/// The ten silhouettes with the upper edge of each band, in order.
const BANDS: [(Kind, f64); 10] = [
    (Kind::Round, 0.22),
    (Kind::Organic, 0.48),
    (Kind::Boxy, 0.6),
    (Kind::Capsule, 0.7),
    (Kind::Nub, 0.79),
    (Kind::Cloud, 0.86),
    (Kind::Droplet, 0.915),
    (Kind::Hexagon, 0.95),
    (Kind::Sun, 0.98),
    (Kind::Triangle, 1.0),
];

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Round,
        Kind::Organic,
        Kind::Boxy,
        Kind::Capsule,
        Kind::Nub,
        Kind::Cloud,
        Kind::Droplet,
        Kind::Hexagon,
        Kind::Sun,
        Kind::Triangle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Round => "round",
            Kind::Organic => "organic",
            Kind::Boxy => "boxy",
            Kind::Capsule => "capsule",
            Kind::Nub => "nub",
            Kind::Cloud => "cloud",
            Kind::Droplet => "droplet",
            Kind::Hexagon => "hexagon",
            Kind::Sun => "sun",
            Kind::Triangle => "triangle",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The `shape` trait at the middle of the silhouette's band, which is how
    /// the official desktop pins one.
    pub fn trait_value(self) -> f64 {
        match self {
            Kind::Round => 0.11,
            Kind::Organic => 0.35,
            Kind::Boxy => 0.54,
            Kind::Capsule => 0.65,
            Kind::Nub => 0.745,
            Kind::Cloud => 0.825,
            Kind::Droplet => 0.8875,
            Kind::Hexagon => 0.9325,
            Kind::Sun => 0.965,
            Kind::Triangle => 0.99,
        }
    }

    fn core(self) -> f64 {
        match self {
            Kind::Round => 1.0,
            Kind::Organic => 0.98,
            Kind::Boxy => 0.86,
            Kind::Capsule => 1.02,
            Kind::Nub => 0.88,
            Kind::Cloud => 0.78,
            Kind::Droplet => 0.78,
            Kind::Hexagon => 1.05,
            Kind::Sun => 0.7,
            Kind::Triangle => 1.15,
        }
    }
}

struct Body {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    n: f64,
    rot: f64,
    radii: Vec<f64>,
    sides: i64,
    round: f64,
}

#[derive(Clone, Copy)]
struct Ellipse {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
}

struct Eye {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    n: f64,
    rot: f64,
}

struct Petal {
    cx: f64,
    cy: f64,
    r: f64,
}

fn shrunk(b: &Body, k: f64) -> Ellipse {
    Ellipse { cx: b.cx, cy: b.cy, rx: b.rx * k, ry: b.ry * k }
}

fn spline_face(b: &Body) -> Ellipse {
    let min = b.radii.iter().copied().fold(f64::INFINITY, f64::min);
    shrunk(b, min * 0.95)
}

fn face_fit(t: &Traits, b: &Body, face: Ellipse) -> [Eye; 2] {
    let rx = b.rx;
    let er0 = t.num("eye.rx", 0.075, 0.105) * rx;
    let ratio = t.num("eye.ratio", 1.9, 3.2);
    let scale = t.num("eye.scale", 0.78, 1.24);
    let stretch = t.num("eye.stretch", 0.85, 1.18);
    let clearance = t.num("eye.gap", 0.1, 0.24) * rx;
    let wide = er0 * scale.max(1.0);
    let tall = er0 * ratio * (scale * stretch).max(1.0);
    let gap0 = wide + rx * 0.03 + clearance;

    let gx = t.jitter("gaze.x", 0.09) * face.rx;
    let gy = t.num("gaze.y", -0.2, 0.08) * face.ry;
    let dy = t.jitter("eye.dy", 0.04) * face.ry;
    let reach = wide.hypot(tall);
    let need = ((gx.abs() + gap0 + reach) / face.rx).hypot((gy.abs() + dy.abs() + reach) / face.ry);
    let fit = if need > 0.9 { 0.9 / need } else { 1.0 };

    let er = er0 * fit;
    let eye_ry = er * ratio;
    let gap = gap0 * fit;
    let room = (clearance / tall).clamp(0.0, 1.0);
    let bound = 12f64.min((room.asin() * 180.0) / PI);
    let lean = t.num("eye.lean", -1.0, 1.0) * bound;
    let lean2 = (lean + t.jitter("eye.lean2", 3.5)).clamp(-12.0, 12.0);

    let cx = face.cx + gx * fit;
    let cy = face.cy + gy * fit;
    [
        Eye { cx: cx - gap, cy, rx: er, ry: eye_ry, n: t.num("eye.n", 3.5, 6.0), rot: lean },
        Eye {
            cx: cx + gap,
            cy: cy + dy * fit,
            rx: er * scale,
            ry: eye_ry * scale * stretch,
            n: t.num("eye.n", 3.5, 6.0),
            rot: lean2,
        },
    ]
}

/// The figure's markup inside `<svg>`, without the root element.
fn figure(t: &Traits, head: &str, eye_fill: &str) -> String {
    let v = t.get("shape");
    let kind = BANDS.iter().find(|(_, up_to)| v < *up_to).map(|(k, _)| *k).unwrap_or(Kind::Triangle);
    let r = t.num("body.r", 31.0, 38.0) * kind.core();
    let mut b = Body {
        cx: 50.0 + t.jitter("body.x", 1.5),
        cy: 50.0 + t.jitter("body.y", 1.5),
        rx: r,
        ry: r * t.num("body.ratio", 0.92, 1.08),
        n: t.num("body.n", 1.9, 2.5),
        rot: 0.0,
        radii: (0..t.int("body.pts", 6, 8)).map(|i| 1.0 + t.jitter(&format!("body.r{i}"), 0.16)).collect(),
        sides: 0,
        round: 0.3,
    };

    match kind {
        Kind::Boxy => {
            b.n = t.num("body.n", 3.4, 6.0);
            b.rot = t.num("body.rot", -20.0, 20.0);
        }
        Kind::Capsule => b.ry *= t.num("capsule.squat", 0.55, 0.68),
        Kind::Droplet => {
            b.cy += 0.22 * b.ry;
            b.n = 2.0;
        }
        Kind::Hexagon => {
            b.sides = 6;
            b.rot = t.num("body.rot", -12.0, 12.0);
            b.round = t.num("poly.round", 0.24, 0.5);
        }
        Kind::Triangle => {
            b.sides = 3;
            b.rot = t.num("body.rot", -5.0, 5.0);
            b.round = t.num("poly.round", 0.24, 0.5);
        }
        _ => {}
    }

    let face = match kind {
        Kind::Organic | Kind::Cloud => spline_face(&b),
        Kind::Capsule => shrunk(&b, 0.94),
        Kind::Droplet => Ellipse { cx: b.cx, cy: b.cy + b.ry * 0.05, rx: b.rx * 0.88, ry: b.ry * 0.88 },
        Kind::Hexagon => shrunk(&b, 0.84),
        Kind::Triangle => Ellipse { cx: b.cx, cy: b.cy + b.ry * 0.1, rx: b.rx * 0.54, ry: b.ry * 0.36 },
        _ => Ellipse { cx: b.cx, cy: b.cy, rx: b.rx, ry: b.ry },
    };

    let mut petals: Vec<Petal> = Vec::new();
    let mut extra: Vec<String> = Vec::new();
    match kind {
        Kind::Capsule => {
            for s in [-1.0, 1.0] {
                petals.push(Petal { cx: b.cx + s * (b.rx - b.ry), cy: b.cy, r: b.ry });
            }
        }
        Kind::Nub => {
            for i in 0..t.int("nub.n", 1, 2) {
                let a = t.num(&format!("nub.a{i}"), 0.0, 2.0 * PI);
                petals.push(Petal {
                    cx: b.cx + a.cos() * b.rx * 0.88,
                    cy: b.cy + a.sin() * b.rx * 0.88,
                    r: b.rx * t.num(&format!("nub.r{i}"), 0.24, 0.4),
                });
            }
        }
        Kind::Cloud => {
            let count = t.int("cloud.n", 4, 6);
            for i in 0..count {
                let a = PI + (PI * (i as f64 + 0.5)) / count as f64;
                petals.push(Petal {
                    cx: b.cx + a.cos() * b.rx * 0.8,
                    cy: b.cy + a.sin() * b.rx * 0.5,
                    r: b.rx * t.num(&format!("cloud.r{i}"), 0.44, 0.62),
                });
            }
        }
        Kind::Droplet => extra.push(taper(b.cx, b.cy, b.rx, b.ry, t.num("droplet.tip", 1.4, 1.65))),
        Kind::Sun => {
            let count = t.int("sun.n", 6, 9);
            let dist = b.rx * t.num("sun.dist", 1.0, 1.08);
            let pr = b.rx * t.num("sun.r", 0.2, 0.26);
            let off = t.num("sun.rot", 0.0, 2.0 * PI);
            for i in 0..count {
                let a = off + (2.0 * PI * i as f64) / count as f64;
                petals.push(Petal { cx: b.cx + a.cos() * dist, cy: b.cy + a.sin() * dist, r: pr });
            }
        }
        _ => {}
    }

    let eyes = face_fit(t, &b, face);

    let core = match kind {
        Kind::Organic | Kind::Cloud => blob_path(b.cx, b.cy, b.rx, b.ry, &b.radii, b.rot),
        Kind::Capsule => box_path(b.cx, b.cy, b.rx - b.ry, b.ry),
        Kind::Hexagon | Kind::Triangle => polygon(b.cx, b.cy, b.rx, b.ry, b.sides, b.round, b.rot),
        _ => superellipse(b.cx, b.cy, b.rx, b.ry, b.n, b.rot),
    };

    let mut out = format!("<g fill=\"{head}\">");
    for p in &petals {
        out += &format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
            js_num(r2n(p.cx)),
            js_num(r2n(p.cy)),
            js_num(r2n(p.r))
        );
    }
    for d in &extra {
        out += &format!("<path d=\"{d}\"/>");
    }
    out += &format!("<path d=\"{core}\"/></g><g fill=\"{eye_fill}\">");
    for e in &eyes {
        out += &format!("<path d=\"{}\"/>", superellipse(e.cx, e.cy, e.rx, e.ry, e.n, e.rot));
    }
    out + "</g>"
}

/// `blobatar(seed, { size, traits: { shape } })` from `blobatar/blob`.
/// `size` 0 leaves out width and height.
pub fn svg(seed: &str, kind: Option<Kind>, size: u32) -> String {
    let t = Traits { state: seed_state(seed), shape: kind.map(Kind::trait_value) };
    let (head, eye) = palette(t.num("hue", 0.0, 360.0), t.get("tone"));
    let dim = if size > 0 { format!(" width=\"{size}\" height=\"{size}\"") } else { String::new() };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"{dim}>{}</svg>",
        figure(&t, &head, &eye)
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// SVGs from the real blobatar 2.7.0 (`node`, `blobatar/blob`), for random
    /// and awkward seeds, with and without a pinned silhouette.
    #[test]
    fn matches_the_javascript_library() {
        let golden: Vec<Value> = serde_json::from_str(include_str!("../../../tests/fixtures/blobatar-2.7.0.json")).unwrap();
        let mut failures = Vec::new();
        for case in &golden {
            let seed = case["seed"].as_str().unwrap();
            let kind = Kind::parse(case["kind"].as_str().unwrap());
            let size = case["size"].as_u64().unwrap() as u32;
            let ours = svg(seed, kind, size);
            if ours != case["svg"].as_str().unwrap() {
                failures.push(format!("{seed:?} {kind:?}\n ours: {ours}\n  lib: {}", case["svg"].as_str().unwrap()));
            }
        }
        assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), golden.len(), failures[..failures.len().min(3)].join("\n"));
    }
}
