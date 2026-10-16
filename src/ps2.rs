//! A small software renderer in the spirit of a PS2 game, for `laifen tui`: the low-poly brush
//! (`wave.rs`) lit per pixel, with an environment-mapped metal, a mirror floor with a
//! projected shadow, rendered at a low resolution and quantised to 15-bit colour with ordered
//! dithering. Adapted from nothing-toolbox's headset renderer (same author).
//!
//! Spaces: the model stands on the floor (Z up, its front towards -Y); the orbit camera turns
//! it into view space: x right, y into the screen, z up.

use crate::kitty::Rgb8;
use crate::wave::{self, Finish, Material, Tri, Wave, FLOOR_Z, MM, SHAFT_BASE};
use std::sync::OnceLock;

type V = [f32; 3];
pub type Mat3 = [[f64; 3]; 3];

/// Camera distance: a mild perspective.
const DISTANCE: f32 = 8.0;
/// Height of the picture's centre.
const LIFT: f32 = 0.0;
/// The scene spans about this many model units across the shorter side of the picture.
const SPAN: f32 = 3.6;
const FLOOR_R: f32 = 2.0;
/// Key light, towards the light (world space: from the upper front left of the viewer).
const KEY: V = [-0.40, -0.65, 0.65];
const FILL: V = [0.75, 0.35, 0.10];

/// What changes from one frame to the next.
#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub finish: Finish,
    /// The head's oscillation about the shaft, in radians.
    pub swing: f64,
    /// The head's vibration: small tilts about X and Y at the shaft's base, in radians.
    pub tilt: [f64; 2],
    /// Which of the three mode lights are lit.
    pub leds: [bool; 3],
    /// The battery light's colour, or None when off.
    pub battery: Option<[f32; 3]>,
}

impl Default for Scene {
    fn default() -> Scene {
        Scene { finish: Finish::Steel, swing: 0.0, tilt: [0.0; 2], leds: [false; 3], battery: None }
    }
}

pub fn matmul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn rot_x(a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(a: f64) -> Mat3 {
    let (c, s) = (a.cos(), a.sin());
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Orbit camera: azimuth about the vertical (Z), then elevation. At 0/0 the camera looks along
/// +Y, at the brush's front.
pub fn view_matrix(az_deg: f64, el_deg: f64) -> Mat3 {
    matmul(&rot_x(el_deg.to_radians()), &rot_z(az_deg.to_radians()))
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: V) -> V {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn mix(a: V, b: V, t: f32) -> V {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn scale(a: V, k: f32) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mat32(m: &Mat3) -> [[f32; 3]; 3] {
    m.map(|r| r.map(|v| v as f32))
}

fn apply(m: &[[f32; 3]; 3], p: V) -> V {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
}

/// sRGB <-> linear, close enough for shading.
fn linear(c: V) -> V {
    c.map(|v| v.max(0.0).powf(2.2))
}

fn model(finish: Finish) -> &'static Wave {
    static STEEL: OnceLock<Wave> = OnceLock::new();
    static ALU: OnceLock<Wave> = OnceLock::new();
    static WHITE: OnceLock<Wave> = OnceLock::new();
    let cell = match finish {
        Finish::Steel => &STEEL,
        Finish::Aluminium => &ALU,
        Finish::White => &WHITE,
    };
    cell.get_or_init(|| wave::model(finish))
}

/// A triangle placed in the world for this frame: corners, vertex normals, material.
struct Placed {
    v: [V; 3],
    n: [V; 3],
    mat: Material,
}

/// Every triangle of the brush for this frame: the head turned and tilted about the shaft,
/// the lights in their current material.
fn placed(scene: &Scene) -> Vec<Placed> {
    let w = model(scene.finish);
    let f = |p: [f64; 3]| p.map(|c| c as f32);
    let mut out = Vec::new();
    let mut put = |tris: &[Tri], mat: Option<Material>, m: Option<&[[f32; 3]; 3]>, pivot: V| {
        for t in tris {
            let (v, n) = match m {
                Some(m) => (t.v.map(|p| add(apply(m, sub(f(p), pivot)), pivot)), t.n.map(|p| apply(m, f(p)))),
                None => (t.v.map(f), t.n.map(f)),
            };
            out.push(Placed { v, n, mat: mat.unwrap_or(t.mat) });
        }
    };
    put(&w.body, None, None, [0.0; 3]);
    let turn = mat32(&matmul(&matmul(&rot_x(scene.tilt[0]), &rot_y(scene.tilt[1])), &rot_z(scene.swing)));
    let pivot = [0.0, 0.0, (SHAFT_BASE * MM + FLOOR_Z) as f32];
    put(&w.head, None, Some(&turn), pivot);
    // A light that is off is invisible, as on the real brush.
    let lamp = |on: Option<[f32; 3]>| match on {
        Some(rgb) => wave::light(Some(rgb)),
        None => wave::shell(scene.finish),
    };
    for (i, led) in w.leds.iter().enumerate() {
        put(led, Some(lamp(scene.leds[i].then_some([0.95, 0.97, 1.0]))), None, [0.0; 3]);
    }
    put(&w.battery, Some(lamp(scene.battery)), None, [0.0; 3]);
    out
}

/// A vertex on screen: position, 1/w for perspective-correct interpolation, and attributes
/// (view-space normal, then a free position used by the floor).
#[derive(Clone, Copy)]
struct Vert {
    x: f32,
    y: f32,
    inv_w: f32,
    a: [f32; 6],
}

/// Fills a triangle: `f(index, inv_w, attributes)` for each covered pixel centre.
fn raster(v: &[Vert; 3], w: usize, h: usize, mut f: impl FnMut(usize, f32, [f32; 6])) {
    let area = (v[1].x - v[0].x) * (v[2].y - v[0].y) - (v[2].x - v[0].x) * (v[1].y - v[0].y);
    if area.abs() < 1e-6 || !area.is_finite() {
        return;
    }
    let min = |k: fn(&Vert) -> f32| v.iter().map(k).fold(f32::MAX, f32::min);
    let max = |k: fn(&Vert) -> f32| v.iter().map(k).fold(f32::MIN, f32::max);
    let x0 = min(|p| p.x).floor().max(0.0) as usize;
    let x1 = (max(|p| p.x).ceil().max(0.0) as usize).min(w);
    let y0 = min(|p| p.y).floor().max(0.0) as usize;
    let y1 = (max(|p| p.y).ceil().max(0.0) as usize).min(h);
    let pre: [[f32; 6]; 3] = v.map(|p| p.a.map(|a| a * p.inv_w));
    for y in y0..y1 {
        let fy = y as f32 + 0.5;
        for x in x0..x1 {
            let fx = x as f32 + 0.5;
            let e = |a: &Vert, b: &Vert| (b.x - a.x) * (fy - a.y) - (fx - a.x) * (b.y - a.y);
            let b0 = e(&v[1], &v[2]) / area;
            let b1 = e(&v[2], &v[0]) / area;
            let b2 = e(&v[0], &v[1]) / area;
            if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                continue;
            }
            let iw = b0 * v[0].inv_w + b1 * v[1].inv_w + b2 * v[2].inv_w;
            let a = [0, 1, 2, 3, 4, 5].map(|k| (b0 * pre[0][k] + b1 * pre[1][k] + b2 * pre[2][k]) / iw);
            f(y * w + x, iw, a);
        }
    }
}

/// The camera: the orbit of the terminal view, the projection, and the lights in view space.
struct Camera {
    view: [[f32; 3]; 3],
    w: usize,
    h: usize,
    /// Horizontal centre of the picture, in pixels.
    cx: f32,
    s: f32,
    key: V,
    fill: V,
    up: V,
}

impl Camera {
    fn new(w: usize, h: usize, az: f64, el: f64, shift: f64) -> Camera {
        let view = mat32(&view_matrix(az, el));
        Camera {
            view,
            w,
            h,
            cx: w as f32 * (0.5 + shift as f32),
            s: w.min(h) as f32 / SPAN,
            key: unit(apply(&view, KEY)),
            fill: unit(apply(&view, FILL)),
            up: apply(&view, [0.0, 0.0, 1.0]),
        }
    }

    /// View space -> screen.
    fn project(&self, p: V, a: [f32; 6]) -> Vert {
        let inv_w = DISTANCE / (DISTANCE + p[1]).max(0.5);
        Vert {
            x: self.cx + p[0] * inv_w * self.s,
            y: self.h as f32 / 2.0 - (p[2] - LIFT) * inv_w * self.s,
            inv_w,
            a,
        }
    }
}

/// The studio around the brush, as seen in a reflection along `r` (view space, `up` is the
/// world's up): a bright horizon and a dark floor, a large white card facing the brush, two
/// tall strip lights on its sides, a soft box towards the key light. A mirror finish shows
/// little but these: the card on the flat front, the strips as bands on the curved sides,
/// as in the vendor's product shots.
fn environment(r: V, cam: &Camera) -> V {
    let up = dot(r, cam.up);
    let sky = mix([0.30, 0.34, 0.42], [0.85, 0.88, 0.95], up.clamp(0.0, 1.0).powf(0.6));
    let ground = mix([0.10, 0.10, 0.12], [0.03, 0.03, 0.04], (-up).clamp(0.0, 1.0).sqrt());
    let mut c = if up >= 0.0 { sky } else { ground };
    c = add(c, scale([1.0, 0.97, 0.92], (1.0 - (up * 9.0).abs()).max(0.0) * 0.7));
    // The card: towards the camera (-Y in view space), wide, slightly above.
    let card = unit([0.0, -1.0, 0.25]);
    c = add(c, scale([0.95, 0.96, 0.98], dot(r, card).max(0.0).powf(3.0) * 1.1));
    // The strips: narrow in azimuth (about the world's vertical), tall.
    let side = [cam.view[0][0], cam.view[1][0], cam.view[2][0]];
    let ahead = [cam.view[0][1], cam.view[1][1], cam.view[2][1]];
    let az = dot(r, side).atan2(-dot(r, ahead));
    for centre in [1.25f32, -1.25] {
        let band = (1.0 - ((az - centre) / 0.16).abs()).max(0.0);
        c = add(c, scale([1.0, 1.0, 1.0], band * (1.0 - up.abs()).max(0.0) * 1.6));
    }
    let k = dot(r, cam.key).max(0.0).powf(12.0);
    add(c, scale([1.2, 1.15, 1.05], k))
}

/// Lit colour (linear) of a surface point; `n` faces the camera.
fn shade(m: &Material, n: V, cam: &Camera) -> V {
    let eye = [0.0, -1.0, 0.0];
    let base = linear(m.rgb);
    if m.glow {
        return base;
    }
    let hemi = 0.5 + 0.5 * dot(n, cam.up);
    let ambient = mix([0.10, 0.10, 0.12], [0.30, 0.32, 0.38], hemi);
    let diffuse = add(
        ambient,
        add(
            scale([1.0, 0.97, 0.92], dot(n, cam.key).max(0.0)),
            scale([0.30, 0.36, 0.48], dot(n, cam.fill).max(0.0)),
        ),
    );
    let half = unit(add(cam.key, eye));
    let spec = m.spec * dot(n, half).max(0.0).powf(m.shine);
    let facing = dot(n, eye).max(0.0);
    let rim = (1.0 - facing).powi(3) * 0.35;
    let mut c = [0, 1, 2].map(|k| base[k] * diffuse[k] + spec + rim * 0.5);
    if m.metal > 0.0 {
        // Reflect the eye ray about the normal.
        let d = [0.0, 1.0, 0.0];
        let r = sub(d, scale(n, 2.0 * dot(d, n)));
        let env = linear(environment(r, cam));
        c = mix(c, [0, 1, 2].map(|k| env[k] * base[k] * 1.3 + spec), m.metal);
    }
    c
}

/// Fixed backdrop: a vertical gradient, darker at the bottom, with a soft vignette.
fn backdrop(x: usize, y: usize, w: usize, h: usize) -> V {
    let t = y as f32 / h.max(1) as f32;
    let top = [0.165, 0.185, 0.235];
    let bottom = [0.040, 0.044, 0.056];
    let dx = x as f32 / w.max(1) as f32 - 0.5;
    let v = 1.0 - 0.35 * (dx * dx * 2.0);
    linear(mix(top, bottom, t.powf(0.8))).map(|c| c * v)
}

/// 4x4 Bayer thresholds for the ordered dither.
const BAYER: [[f32; 4]; 4] =
    [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// Quantises to 5 bits per channel (the PS2's 16-bit frame buffer), dithered.
fn quantise(c: V, x: usize, y: usize) -> [u8; 3] {
    let t = (BAYER[y % 4][x % 4] + 0.5) / 16.0;
    c.map(|v| {
        let s = v.max(0.0).powf(1.0 / 2.2).min(1.0) * 31.0;
        let q = (s.floor() + if s.fract() > t { 1.0 } else { 0.0 }).min(31.0);
        (q * 255.0 / 31.0).round() as u8
    })
}

struct Frame {
    color: Vec<V>,
    depth: Vec<f32>,
}

/// The brush in view space, optionally mirrored about the floor.
fn to_view(tris: &[Placed], cam: &Camera, mirror: bool) -> Vec<Placed> {
    // The floor is horizontal in the world: in view space, the plane through the floor's
    // centre with normal `up`.
    let floor = apply(&cam.view, [0.0, 0.0, FLOOR_Z as f32]);
    let point = |p: V| if mirror { sub(p, scale(cam.up, 2.0 * dot(sub(p, floor), cam.up))) } else { p };
    let normal = |n: V| if mirror { sub(n, scale(cam.up, 2.0 * dot(n, cam.up))) } else { n };
    tris.iter()
        .map(|t| Placed {
            v: t.v.map(|p| point(apply(&cam.view, p))),
            n: t.n.map(|p| normal(apply(&cam.view, p))),
            mat: t.mat,
        })
        .collect()
}

/// Draws triangles with a depth test into `fb`.
fn draw_model(tris: &[Placed], cam: &Camera, fb: &mut Frame) {
    for t in tris {
        // Two-sided: turn the normals towards the camera when the face looks away. Judged
        // from the normals, not the winding, which the mirror view reverses.
        let flip = dot(add(add(t.n[0], t.n[1]), t.n[2]), [0.0, -1.0, 0.0]) < 0.0;
        let verts = [0, 1, 2].map(|i| {
            let nn = if flip { scale(t.n[i], -1.0) } else { t.n[i] };
            cam.project(t.v[i], [nn[0], nn[1], nn[2], 0.0, 0.0, 0.0])
        });
        raster(&verts, cam.w, cam.h, |i, iw, a| {
            if iw <= fb.depth[i] {
                return;
            }
            fb.depth[i] = iw;
            fb.color[i] = shade(&t.mat, unit([a[0], a[1], a[2]]), cam);
        });
    }
}

/// Renders the brush for an orbit camera, `w` x `h` pixels.
#[cfg(test)]
pub fn render(scene: &Scene, w: usize, h: usize, az: f64, el: f64) -> Rgb8 {
    render_shifted(scene, w, h, az, el, 0.0)
}

/// `render`, with the brush moved sideways by `shift` times the picture's width.
pub fn render_shifted(scene: &Scene, w: usize, h: usize, az: f64, el: f64, shift: f64) -> Rgb8 {
    let cam = Camera::new(w, h, az, el, shift);
    let world = placed(scene);
    let mut fb =
        Frame { color: (0..w * h).map(|i| backdrop(i % w, i / w, w, h)).collect(), depth: vec![0.0; w * h] };
    let floor_z = FLOOR_Z as f32;

    // Mirror floor, while the camera is above it.
    let eye_z = -DISTANCE * cam.view[1][2];
    if eye_z > floor_z + 0.3 {
        let fade = ((eye_z - floor_z - 0.3) / 0.8).min(1.0);
        // The reflection: the brush mirrored about the floor, in its own buffer.
        let mut refl = Frame { color: fb.color.clone(), depth: vec![0.0; w * h] };
        draw_model(&to_view(&world, &cam, true), &cam, &mut refl);

        // The floor: a disc of quads, faded into the backdrop at its rim.
        let (rings, segs) = (6, 40);
        let at = |r: f32, t: f32| {
            let p = [r * t.cos(), r * t.sin(), floor_z];
            cam.project(apply(&cam.view, p), [0.0, 0.0, 0.0, p[0], p[1], 0.0])
        };
        let mut shadow = vec![false; w * h];
        let mut floor_px = vec![false; w * h];
        let mut ground = vec![[0.0f32; 2]; w * h];
        for i in 0..rings {
            for j in 0..segs {
                let (r0, r1) = (FLOOR_R * i as f32 / rings as f32, FLOOR_R * (i + 1) as f32 / rings as f32);
                let (t0, t1) = (
                    std::f32::consts::TAU * j as f32 / segs as f32,
                    std::f32::consts::TAU * (j + 1) as f32 / segs as f32,
                );
                for tri in [[at(r0, t0), at(r1, t0), at(r1, t1)], [at(r0, t0), at(r1, t1), at(r0, t1)]] {
                    raster(&tri, w, h, |i, iw, a| {
                        fb.depth[i] = iw;
                        floor_px[i] = true;
                        ground[i] = [a[3], a[4]];
                    });
                }
            }
        }
        // The shadow: the brush squashed onto the floor along the key light.
        for t in &world {
            let v = t.v.map(|p| {
                let k = (p[2] - floor_z) / KEY[2];
                let g = [p[0] - KEY[0] * k * 0.5, p[1] - KEY[1] * k * 0.5, floor_z];
                cam.project(apply(&cam.view, g), [0.0; 6])
            });
            raster(&v, w, h, |i, _, _| shadow[i] = true);
        }
        // Shade the floor pixels: reflection, tint, grid, shadow, fade.
        let floor_rgb = linear([0.075, 0.080, 0.095]);
        let grid_rgb = linear([0.16, 0.20, 0.27]);
        for i in 0..w * h {
            if !floor_px[i] {
                continue;
            }
            let [x, y] = ground[i];
            let r = (x * x + y * y).sqrt() / FLOOR_R;
            let line = |v: f32| {
                let f = (v / 0.5).rem_euclid(1.0);
                !(0.04..=0.96).contains(&f)
            };
            let mut c = mix(refl.color[i], floor_rgb, 0.55 + 0.35 * r);
            if line(x) || line(y) {
                c = mix(c, grid_rgb, 0.8 * (1.0 - r));
            }
            if shadow[i] {
                c = scale(c, 0.35 + 0.4 * r);
            }
            let edge = ((1.0 - r) / 0.35).clamp(0.0, 1.0) * fade;
            fb.color[i] = mix(fb.color[i], c, edge);
        }
    }

    draw_model(&to_view(&world, &cam, false), &cam, &mut fb);

    let mut px = Vec::with_capacity(w * h * 3);
    for (i, c) in fb.color.iter().enumerate() {
        px.extend(quantise(*c, i % w, i / w));
    }
    Rgb8 { w, h, px }
}

/// The brush alone, on a transparent background (RGBA, no floor): for the README's banner.
#[cfg(test)]
pub fn render_rgba(scene: &Scene, w: usize, h: usize, az: f64, el: f64) -> Vec<u8> {
    let cam = Camera::new(w, h, az, el, 0.0);
    let mut fb = Frame { color: vec![[0.0; 3]; w * h], depth: vec![0.0; w * h] };
    draw_model(&to_view(&placed(scene), &cam, false), &cam, &mut fb);
    let mut px = Vec::with_capacity(w * h * 4);
    for (i, c) in fb.color.iter().enumerate() {
        px.extend(quantise(*c, i % w, i / w));
        px.push(if fb.depth[i] > 0.0 { 255 } else { 0 });
    }
    px
}

/// Scales a picture up by a whole factor, each pixel becoming a `k` x `k` block, so that the
/// terminal (which smooths what it scales) has almost nothing left to scale.
pub fn upscale(img: &Rgb8, k: usize) -> Rgb8 {
    if k <= 1 {
        return Rgb8 { w: img.w, h: img.h, px: img.px.clone() };
    }
    let (w, h) = (img.w * k, img.h * k);
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..img.h {
        let start = px.len();
        for x in 0..img.w {
            let p = &img.px[(y * img.w + x) * 3..][..3];
            for _ in 0..k {
                px.extend_from_slice(p);
            }
        }
        for _ in 1..k {
            px.extend_from_within(start..start + w * 3);
        }
    }
    Rgb8 { w, h, px }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::zlib;

    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// A PNG file of `img`, to look at the renderer's output.
    pub fn png(img: &Rgb8) -> Vec<u8> {
        png_of(img.w, img.h, 3, &img.px)
    }

    /// A PNG file of `w` x `h` pixels of `channels` bytes (3: RGB, 4: RGBA).
    pub fn png_of(w: usize, h: usize, channels: usize, px: &[u8]) -> Vec<u8> {
        let mut raw = Vec::with_capacity((w * channels + 1) * h);
        for row in px.chunks(w * channels) {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend((w as u32).to_be_bytes());
        ihdr.extend((h as u32).to_be_bytes());
        ihdr.extend([8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &zlib::compress(&raw, w * channels + 1));
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn the_brush_is_drawn_in_the_middle() {
        // The centre column holds the brush: lighter than the dark backdrop around it.
        let img = render(&Scene::default(), 160, 120, 20.0, 10.0);
        let lum =
            |x: usize, y: usize| img.px[(y * 160 + x) * 3..][..3].iter().map(|&v| v as u32).sum::<u32>();
        let centre: u32 = (30..90).map(|y| lum(80, y)).sum();
        let side: u32 = (30..90).map(|y| lum(10, y)).sum();
        assert!(centre > side * 2, "centre {centre}, side {side}");
    }

    #[test]
    fn the_head_swings_and_the_lights_light() {
        let a = render(&Scene::default(), 120, 120, 0.0, 10.0);
        let b = render(&Scene { swing: 0.5, ..Scene::default() }, 120, 120, 0.0, 10.0);
        let c = render(&Scene { leds: [true, false, false], ..Scene::default() }, 120, 120, 0.0, 10.0);
        assert_ne!(a.px, b.px);
        assert_ne!(a.px, c.px);
    }

    /// The README's pictures, to $LAIFEN_PREVIEW_DIR: the brush turning once on a transparent
    /// background, small, for the banner to scale up in whole pixels (`spin/`); the picture
    /// behind the window of `laifen tui` for a 150 x 42 terminal with the cells of the README's
    /// screenshots, 7.5 x 20 pixels (`tui.png`); the same while brushing with the owner's Mode 2
    /// (strength, range and speed 10), 10 frames a second (`brushing/`).
    #[test]
    #[ignore]
    fn readme_pictures() {
        let dir = std::env::var("LAIFEN_PREVIEW_DIR").expect("LAIFEN_PREVIEW_DIR");
        let lit = Scene { leds: [false, true, false], battery: Some([0.35, 0.95, 0.45]), ..Scene::default() };
        std::fs::create_dir_all(format!("{dir}/spin")).unwrap();
        for i in 0..64 {
            let (w, h) = (220, 220);
            let px = render_rgba(&lit, w, h, i as f64 * 360.0 / 64.0, 8.0);
            std::fs::write(format!("{dir}/spin/frame-{i:03}.png"), png_of(w, h, 4, &px)).unwrap();
        }
        // As `KittyView` draws it: rendered at about 200 lines, scaled up by a whole factor.
        let (cols, rows, k) = (150usize, 42usize, 4usize);
        let (pw, ph) = (cols * 15 / 2 / k, rows * 20 / k);
        let shift = -(crate::viz::WINDOW as f64) / 2.0 / cols as f64;
        let img = upscale(&render_shifted(&lit, pw, ph, 25.0, 10.0, shift), k);
        std::fs::write(format!("{dir}/tui.png"), png(&img)).unwrap();
        std::fs::create_dir_all(format!("{dir}/brushing")).unwrap();
        for i in 0..40u64 {
            let t = i as f64 / 10.0;
            let (amplitude, freq) = (30f64.to_radians(), 0.8 + 0.22 * 10.0);
            let n = |seed: u64| {
                ((i.wrapping_mul(6364136223846793005).wrapping_add(seed) >> 33) as f64 / (1u64 << 31) as f64)
                    * 2.0
                    - 1.0
            };
            let scene = Scene {
                swing: amplitude * (std::f64::consts::TAU * freq * t).sin(),
                tilt: [0.9f64.to_radians() * n(1), 0.9f64.to_radians() * n(7)],
                ..lit
            };
            let img = upscale(&render_shifted(&scene, pw, ph, 25.0 + 1.4 * i as f64, 10.0, shift), k);
            std::fs::write(format!("{dir}/brushing/frame-{i:03}.png"), png(&img)).unwrap();
        }
    }

    /// Writes PNG previews to $LAIFEN_PREVIEW_DIR: each finish, from a few angles.
    #[test]
    #[ignore]
    fn previews() {
        let dir = std::env::var("LAIFEN_PREVIEW_DIR").expect("LAIFEN_PREVIEW_DIR");
        for (i, finish) in [Finish::Steel, Finish::Aluminium, Finish::White].into_iter().enumerate() {
            for (j, az) in [0.0, 35.0, 90.0, 200.0].into_iter().enumerate() {
                let scene = Scene {
                    finish,
                    leds: [true, true, false],
                    battery: Some([0.35, 0.95, 0.45]),
                    ..Scene::default()
                };
                let img = upscale(&render(&scene, 320, 240, az, 12.0), 3);
                std::fs::write(format!("{dir}/wave-{i}-{j}.png"), png(&img)).unwrap();
            }
        }
    }
}
