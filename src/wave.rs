//! A low-poly Laifen Wave (LFTB01-P) for the 3D view (`ps2.rs`), built from the published
//! size of the handle (about 120 mm tall without the shaft, 28 x 32 mm across) and from the
//! vendor's product pictures (proportions, to a few millimetres).
//!
//! Frame, in millimetres before scaling: Z up from the bottom of the handle, X across the
//! front face, Y front to back; the front face, with the button, looks towards -Y.
//!
//! The handle is a squarish oval in section, flat at the front and back, with round shoulders
//! under a light collar. On its front: three small mode lights above a round button, a battery
//! light near the bottom. The motor shaft carries the brush head (a tapered neck, an oval
//! plate, tufts of bristles on the front side): the shaft and the head are the part that
//! oscillates, so they are kept apart from the rest.
//!
//! The mesh helpers (`Builder`) come from nothing-toolbox's low-poly headset (same author).

pub type V3 = [f64; 3];

/// Model units per millimetre: the whole brush, 205 mm, is about 2.6 units tall.
pub const MM: f64 = 1.0 / 80.0;
/// Height of the floor in model units: the handle stands on it.
pub const FLOOR_Z: f64 = -1.25;
/// The shaft's axis is the Z axis; the head tilts (vibration) about this height, in mm.
pub const SHAFT_BASE: f64 = 125.0;

/// The handle's finish. The LFTB01-P was sold in polished stainless steel, aluminium, and
/// white plastic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    Steel,
    Aluminium,
    White,
}

impl Finish {
    pub fn name(self) -> &'static str {
        match self {
            Finish::Steel => "stainless steel",
            Finish::Aluminium => "aluminium",
            Finish::White => "white",
        }
    }

    pub fn next(self) -> Finish {
        match self {
            Finish::Steel => Finish::Aluminium,
            Finish::Aluminium => Finish::White,
            Finish::White => Finish::Steel,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// sRGB, 0..1.
    pub rgb: [f32; 3],
    /// Highlight strength and sharpness.
    pub spec: f32,
    pub shine: f32,
    /// How much of the colour is the reflected studio (metal).
    pub metal: f32,
    /// Drawn at full colour whatever the light (the lit lights).
    pub glow: bool,
}

pub struct Tri {
    pub v: [V3; 3],
    /// Vertex normals: shared across a curved surface, the face's own across a crease.
    pub n: [V3; 3],
    pub mat: Material,
}

/// The parts drawn with a material chosen per frame are kept apart.
pub struct Wave {
    /// Handle, collar, button: never moves.
    pub body: Vec<Tri>,
    /// Shaft and brush head: oscillates about the Z axis.
    pub head: Vec<Tri>,
    /// The three mode lights, left to right.
    pub leds: [Vec<Tri>; 3],
    /// The battery light.
    pub battery: Vec<Tri>,
}

fn hex(c: u32) -> [f32; 3] {
    [(c >> 16) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0]
}

const fn plain(rgb: [f32; 3], spec: f32, shine: f32, metal: f32) -> Material {
    Material { rgb, spec, shine, metal, glow: false }
}

/// A light: its colour when lit (glowing), or dark glass.
pub fn light(on: Option<[f32; 3]>) -> Material {
    match on {
        Some(rgb) => Material { rgb, spec: 0.0, shine: 1.0, metal: 0.0, glow: true },
        None => plain(hex(0x2a2c30), 0.6, 30.0, 0.0),
    }
}

/// The handle's own material: also what a light that is off looks like (it disappears under
/// the surface, as on the real brush).
pub fn shell(f: Finish) -> Material {
    match f {
        // Mirror polish: mostly the reflected studio, a sharp highlight.
        Finish::Steel => plain(hex(0xd4d6da), 1.0, 70.0, 0.92),
        Finish::Aluminium => plain(hex(0xc3c6cb), 0.35, 16.0, 0.45),
        Finish::White => plain(hex(0xeceeef), 0.25, 12.0, 0.0),
    }
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l < 1e-12 {
        a
    } else {
        mul(a, 1.0 / l)
    }
}

/// A rounded rectangle `w` x `h` (corner radius `r`), `n` segments per corner,
/// counter-clockwise from the right side.
fn rrect(w: f64, h: f64, r: f64, n: usize) -> Vec<(f64, f64)> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (cx, cy) = (w / 2.0 - r, h / 2.0 - r);
    let mut out = Vec::with_capacity(4 * (n + 1));
    for (k, (sx, sy)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].iter().enumerate() {
        for i in 0..=n {
            let t = std::f64::consts::FRAC_PI_2 * (k as f64 + i as f64 / n.max(1) as f64);
            out.push((sx * cx + r * t.cos(), sy * cy + r * t.sin()));
        }
    }
    out
}

/// A superellipse `w` x `h` (`power` 2 is an ellipse, higher is squarer), `n` points,
/// counter-clockwise from the right side.
fn oval(w: f64, h: f64, power: f64, n: usize) -> Vec<(f64, f64)> {
    let pw = |v: f64| v.signum() * v.abs().powf(2.0 / power);
    (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            (w / 2.0 * pw(t.cos()), h / 2.0 * pw(t.sin()))
        })
        .collect()
}

/// A local frame: `at(a, b, c) = o + a u + b v + c w`, in millimetres.
#[derive(Clone, Copy)]
struct Frame {
    o: V3,
    u: V3,
    v: V3,
    w: V3,
}

impl Frame {
    fn at(&self, a: f64, b: f64, c: f64) -> V3 {
        add(self.o, add(mul(self.u, a), add(mul(self.v, b), mul(self.w, c))))
    }
}

/// Horizontal sections stacked along Z: `at(x, y, z)`.
const UP: Frame = Frame { o: [0.0; 3], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0], w: [0.0, 0.0, 1.0] };

#[derive(Default)]
struct Builder {
    tris: Vec<Tri>,
}

impl Builder {
    fn tri(&mut self, v: [V3; 3], n: [V3; 3], mat: Material) {
        let place = |p: V3| [p[0] * MM, p[1] * MM, p[2] * MM + FLOOR_Z];
        self.tris.push(Tri { v: v.map(place), n, mat });
    }

    /// A surface through rows of points (rows along the length, points around): quads between
    /// neighbours, `closed` joining the last point of a row to the first. Normals point away
    /// from each row's centre; `smooth` shares them between faces, else each face is flat.
    fn grid(&mut self, rows: &[Vec<V3>], closed: bool, smooth: bool, mat: Material) {
        let n = rows[0].len();
        let quads = if closed { n } else { n - 1 };
        let centre = |r: &Vec<V3>| mul(r.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / r.len() as f64);
        let face = |i: usize, j: usize| {
            let (a, b, c, d) = (rows[i][j], rows[i][(j + 1) % n], rows[i + 1][(j + 1) % n], rows[i + 1][j]);
            let mid = mul(add(add(a, b), add(c, d)), 0.25);
            let axis = mul(add(centre(&rows[i]), centre(&rows[i + 1])), 0.5);
            let mut f = cross(sub(c, a), sub(d, b));
            // A collapsed quad (the tip of a taper) faces away from the axis.
            f = unit(if dot(f, f) < 1e-12 { sub(mid, axis) } else { f });
            if dot(f, sub(mid, axis)) < 0.0 {
                f = mul(f, -1.0);
            }
            f
        };
        let faces: Vec<Vec<V3>> =
            (0..rows.len() - 1).map(|i| (0..quads).map(|j| face(i, j)).collect()).collect();
        let normal = |i: usize, j: usize, fi: usize, fj: usize| {
            if !smooth {
                return faces[fi][fj];
            }
            let mut s = [0.0; 3];
            for di in [i.wrapping_sub(1), i] {
                for dj in [(j + quads - 1) % quads.max(1), j % quads.max(1)] {
                    if di < faces.len() && (closed || (dj + 1 == j || dj == j) && dj < quads) {
                        s = add(s, faces[di][dj]);
                    }
                }
            }
            if dot(s, s) < 1e-12 {
                faces[fi][fj]
            } else {
                unit(s)
            }
        };
        for i in 0..rows.len() - 1 {
            for j in 0..quads {
                let k = (j + 1) % n;
                let v = [rows[i][j], rows[i][k], rows[i + 1][k], rows[i + 1][j]];
                let nn =
                    [normal(i, j, i, j), normal(i, k, i, j), normal(i + 1, k, i, j), normal(i + 1, j, i, j)];
                self.tri([v[0], v[1], v[2]], [nn[0], nn[1], nn[2]], mat);
                self.tri([v[0], v[2], v[3]], [nn[0], nn[2], nn[3]], mat);
            }
        }
    }

    /// A flat polygon (convex outline) in plane `c` of frame `f`, facing `w * facing`.
    fn cap(&mut self, f: &Frame, outline: &[(f64, f64)], c: f64, facing: f64, mat: Material) {
        let n = mul(f.w, facing);
        let mid = outline.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let mid = f.at(mid.0 / outline.len() as f64, mid.1 / outline.len() as f64, c);
        for i in 0..outline.len() {
            let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
            self.tri([mid, f.at(a.0, a.1, c), f.at(b.0, b.1, c)], [n; 3], mat);
        }
    }

    /// Rings of an outline along `w` of frame `f`: (c, inset) pairs, each ring the outline
    /// shrunk by inset (per side); optional flat caps at both ends.
    fn loft(
        &mut self,
        f: &Frame,
        outline: &dyn Fn(f64) -> Vec<(f64, f64)>,
        rings: &[(f64, f64)],
        caps: (bool, bool),
        mat: Material,
    ) {
        let rows: Vec<Vec<V3>> =
            rings.iter().map(|&(c, e)| outline(e).iter().map(|p| f.at(p.0, p.1, c)).collect()).collect();
        self.grid(&rows, true, true, mat);
        let (first, last) = (rings[0], rings[rings.len() - 1]);
        let dir = (last.0 - first.0).signum();
        if caps.0 {
            self.cap(f, &outline(first.1), first.0, -dir, mat);
        }
        if caps.1 {
            self.cap(f, &outline(last.1), last.0, dir, mat);
        }
    }

    /// A round disc of radius `r` on the front face, `proud` mm in front of it.
    fn front_disc(&mut self, x: f64, z: f64, r: f64, proud: f64, mat: Material) {
        let f = Frame {
            o: [x, -(DEPTH / 2.0) - proud, z],
            u: [1.0, 0.0, 0.0],
            v: [0.0, 0.0, 1.0],
            w: [0.0, -1.0, 0.0],
        };
        self.cap(&f, &oval(2.0 * r, 2.0 * r, 2.0, 16), 0.0, 1.0, mat);
    }
}

/// The handle's section, and its height without the collar.
const WIDTH: f64 = 28.2;
const DEPTH: f64 = 31.6;
const SQUARENESS: f64 = 3.2;
const HANDLE: f64 = 120.0;

/// The handle: section rings from the rounded bottom edge to the shoulders.
fn handle(b: &mut Builder, finish: Finish) {
    let section = |e: f64| oval(WIDTH - 2.0 * e, DEPTH - 2.0 * e, SQUARENESS, 40);
    let rings = [
        (0.0, 1.6),
        (0.5, 0.6),
        (1.6, 0.0),
        (40.0, 0.0),
        (80.0, 0.0),
        (110.0, 0.0),
        (113.5, 0.5),
        (116.0, 1.5),
        (118.0, 3.1),
        (119.3, 5.0),
        (HANDLE, 7.2),
    ];
    b.loft(&UP, &section, &rings, (true, true), shell(finish));
}

/// The light collar on top, round, where the shaft comes out.
fn collar(b: &mut Builder) {
    let mat = plain(hex(0xdfe1e3), 0.35, 18.0, 0.0);
    let round = |e: f64| oval(13.6 - 2.0 * e, 13.6 - 2.0 * e, 2.0, 24);
    b.loft(
        &UP,
        &round,
        &[(HANDLE - 1.5, 0.0), (SHAFT_BASE - 1.2, 0.0), (SHAFT_BASE, 1.1)],
        (false, true),
        mat,
    );
}

/// Shaft, neck, head and bristles: the part that oscillates.
fn head(b: &mut Builder) {
    let steel = plain(hex(0xcfd2d6), 0.9, 50.0, 0.8);
    let white = plain(hex(0xf2f3f4), 0.3, 14.0, 0.0);
    let band = plain(hex(0xb9bcc1), 0.5, 22.0, 0.2);
    let circle = |d: f64| move |e: f64| oval(d - 2.0 * e, d - 2.0 * e, 2.0, 16);
    // The shaft, mostly hidden in the neck.
    b.loft(&UP, &circle(4.0), &[(SHAFT_BASE, 0.0), (SHAFT_BASE + 9.0, 0.0)], (false, false), steel);
    // The neck's base, a grey band, then the neck tapering towards the head.
    b.loft(&UP, &circle(11.0), &[(131.0, 0.6), (132.0, 0.0), (138.0, 0.0)], (true, false), white);
    b.loft(&UP, &circle(11.2), &[(138.0, 0.0), (141.0, 0.0)], (false, false), band);
    let neck = |e: f64| oval(10.0 - 2.0 * e, 8.8 - 2.0 * e, 2.4, 20);
    let rings = [(141.0, 0.0), (152.0, 0.9), (166.0, 1.6), (178.0, 1.9)];
    b.loft(&UP, &neck, &rings, (false, false), white);
    // The head: an oval plate, its front at y = -2.6, rounded at the top.
    let plate = |e: f64| oval(11.0 - 2.0 * e, 5.2 - 2.0 * e, 2.6, 20);
    let rings = [(178.0, 0.6), (181.0, 0.0), (201.0, 0.0), (203.5, 1.0), (204.8, 2.2)];
    b.loft(&UP, &plate, &rings, (false, true), white);
    // Bristles: tufts on the front, in three columns, grey and white rows.
    let grey = plain(hex(0xaeb3ba), 0.4, 20.0, 0.0);
    let pale = plain(hex(0xe4e7eb), 0.4, 20.0, 0.0);
    for row in 0..7 {
        let z = 183.0 + row as f64 * 3.1;
        for col in [-3.3, 0.0, 3.3] {
            let tuft =
                Frame { o: [col, -2.4, z], u: [1.0, 0.0, 0.0], v: [0.0, 0.0, 1.0], w: [0.0, -1.0, 0.0] };
            let ring = |e: f64| oval(2.3 - 2.0 * e, 2.3 - 2.0 * e, 2.0, 6);
            let mat = if row % 2 == 0 { pale } else { grey };
            b.loft(&tuft, &ring, &[(0.0, 0.0), (9.0, 0.0), (9.6, 0.4)], (false, true), mat);
        }
    }
}

/// The finished model for `finish`.
pub fn model(finish: Finish) -> Wave {
    let mut body = Builder::default();
    handle(&mut body, finish);
    collar(&mut body);
    // The button: a thin, slightly darker ring round a disc of the handle's own finish.
    body.front_disc(0.0, 89.0, 4.6, 0.12, plain(hex(0x8f949b), 0.8, 40.0, 0.7));
    body.front_disc(0.0, 89.0, 3.8, 0.24, shell(finish));

    let mut head_part = Builder::default();
    head(&mut head_part);

    let led = |x: f64| {
        let mut b = Builder::default();
        b.front_disc(x, 101.0, 1.1, 0.2, light(None));
        b.tris
    };
    let mut battery = Builder::default();
    let dash = Frame {
        o: [0.0, -(DEPTH / 2.0) - 0.2, 32.0],
        u: [1.0, 0.0, 0.0],
        v: [0.0, 0.0, 1.0],
        w: [0.0, -1.0, 0.0],
    };
    battery.cap(&dash, &rrect(3.4, 1.0, 0.5, 2), 0.0, 1.0, light(None));

    Wave {
        body: body.tris,
        head: head_part.tris,
        leds: [led(-3.6), led(0.0), led(3.6)],
        battery: battery.tris,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(tris: &[Tri], axis: usize) -> (f64, f64) {
        let it = tris.iter().flat_map(|t| t.v.iter().map(move |p| p[axis]));
        it.clone().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }

    #[test]
    fn the_model_has_the_brush_s_size() {
        let w = model(Finish::Steel);
        let (z0, _) = extent(&w.body, 2);
        let (_, z1) = extent(&w.head, 2);
        // Stands on the floor, about 205 mm tall overall.
        assert!((z0 - FLOOR_Z).abs() < 1e-9);
        assert!(((z1 - z0) / MM - 204.8).abs() < 0.5);
        let (x0, x1) = extent(&w.body, 0);
        assert!(((x1 - x0) / MM - WIDTH).abs() < 0.2);
    }

    #[test]
    fn bristles_and_controls_face_the_front() {
        let w = model(Finish::Steel);
        // The front is -Y: the tufts reach out that way, the lights sit on the front face.
        let (y0, _) = extent(&w.head, 1);
        assert!(y0 / MM < -11.0);
        for led in &w.leds {
            assert!(extent(led, 1).1 / MM < -(DEPTH / 2.0));
        }
    }
}
