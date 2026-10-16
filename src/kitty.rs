// Taken from nothing-toolbox (same author, MIT): https://github.com/marc-alexis-com/nothing-toolbox
//! The kitty graphics protocol, as used by `laifen tui` to show real pictures.
//!
//! Images are displayed through Unicode placeholders: the picture is sent with a virtual
//! placement (`U=1`) of `c` x `r` cells, then those cells are filled with the character
//! U+10EEEE, whose foreground colour carries the image id and whose combining marks carry the
//! row and column. Being plain text, the placeholders survive tmux, which only has to pass the
//! graphics commands through (`allow-passthrough on`, wrapped in `ESC P tmux; ... ESC \`).
//!
//! Each new frame is sent under the other of two ids, the placeholders are switched to it,
//! then the previous image is deleted: the screen never shows a half-loaded picture.

use crate::zlib;
use std::process::Command;

/// Row and column numbers, as combining marks (kitty's `rowcolumn-diacritics.txt`).
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030d, 0x030e, 0x0310, 0x0312, 0x033d, 0x033e, 0x033f, 0x0346, 0x034a, 0x034b, 0x034c, 0x0350,
    0x0351, 0x0352, 0x0357, 0x035b, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369, 0x036a, 0x036b,
    0x036c, 0x036d, 0x036e, 0x036f, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595,
    0x0597, 0x0598, 0x0599, 0x059c, 0x059d, 0x059e, 0x059f, 0x05a0, 0x05a1, 0x05a8, 0x05a9, 0x05ab, 0x05ac,
    0x05af, 0x05c4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615, 0x0616, 0x0617, 0x0657, 0x0658, 0x0659,
    0x065a, 0x065b, 0x065d, 0x065e, 0x06d6, 0x06d7, 0x06d8, 0x06d9, 0x06da, 0x06db, 0x06dc, 0x06df, 0x06e0,
    0x06e1, 0x06e2, 0x06e4, 0x06e7, 0x06e8, 0x06eb, 0x06ec, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073a,
    0x073d, 0x073f, 0x0740, 0x0741, 0x0743, 0x0745, 0x0747, 0x0749, 0x074a, 0x07eb, 0x07ec, 0x07ed, 0x07ee,
    0x07ef, 0x07f0, 0x07f1, 0x07f3, 0x0816, 0x0817, 0x0818, 0x0819, 0x081b, 0x081c, 0x081d, 0x081e, 0x081f,
    0x0820, 0x0821, 0x0822, 0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082a, 0x082b, 0x082c, 0x082d, 0x0951,
    0x0953, 0x0954, 0x0f82, 0x0f83, 0x0f86, 0x0f87, 0x135d, 0x135e, 0x135f, 0x17dd, 0x193a, 0x1a17, 0x1a75,
    0x1a76, 0x1a77, 0x1a78, 0x1a79, 0x1a7a, 0x1a7b, 0x1a7c, 0x1b6b, 0x1b6d, 0x1b6e, 0x1b6f, 0x1b70, 0x1b71,
    0x1b72, 0x1b73, 0x1cd0, 0x1cd1, 0x1cd2, 0x1cda, 0x1cdb, 0x1ce0, 0x1dc0, 0x1dc1, 0x1dc3, 0x1dc4, 0x1dc5,
    0x1dc6, 0x1dc7, 0x1dc8, 0x1dc9, 0x1dcb, 0x1dcc, 0x1dd1, 0x1dd2, 0x1dd3, 0x1dd4, 0x1dd5, 0x1dd6, 0x1dd7,
    0x1dd8, 0x1dd9, 0x1dda, 0x1ddb, 0x1ddc, 0x1ddd, 0x1dde, 0x1ddf, 0x1de0, 0x1de1, 0x1de2, 0x1de3, 0x1de4,
    0x1de5, 0x1de6, 0x1dfe, 0x20d0, 0x20d1, 0x20d4, 0x20d5, 0x20d6, 0x20d7, 0x20db, 0x20dc, 0x20e1, 0x20e7,
    0x20e9, 0x20f0, 0x2cef, 0x2cf0, 0x2cf1, 0x2de0, 0x2de1, 0x2de2, 0x2de3, 0x2de4, 0x2de5, 0x2de6, 0x2de7,
    0x2de8, 0x2de9, 0x2dea, 0x2deb, 0x2dec, 0x2ded, 0x2dee, 0x2def, 0x2df0, 0x2df1, 0x2df2, 0x2df3, 0x2df4,
    0x2df5, 0x2df6, 0x2df7, 0x2df8, 0x2df9, 0x2dfa, 0x2dfb, 0x2dfc, 0x2dfd, 0x2dfe, 0x2dff, 0xa66f, 0xa67c,
    0xa67d, 0xa6f0, 0xa6f1, 0xa8e0, 0xa8e1, 0xa8e2, 0xa8e3, 0xa8e4, 0xa8e5, 0xa8e6, 0xa8e7, 0xa8e8, 0xa8e9,
    0xa8ea, 0xa8eb, 0xa8ec, 0xa8ed, 0xa8ee, 0xa8ef, 0xa8f0, 0xa8f1, 0xaab0, 0xaab2, 0xaab3, 0xaab7, 0xaab8,
    0xaabe, 0xaabf, 0xaac1, 0xfe20, 0xfe21, 0xfe22, 0xfe23, 0xfe24, 0xfe25, 0xfe26, 0x10a0f, 0x10a38,
    0x1d185, 0x1d186, 0x1d187, 0x1d188, 0x1d189, 0x1d1aa, 0x1d1ab, 0x1d1ac, 0x1d1ad, 0x1d242, 0x1d243,
    0x1d244,
];

const PLACEHOLDER: char = '\u{10EEEE}';
/// Base64 bytes per graphics command (the protocol's limit).
const CHUNK: usize = 4096;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                out.push(B64[(n >> (18 - 6 * k)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn diacritic(n: usize) -> char {
    char::from_u32(DIACRITICS[n.min(DIACRITICS.len() - 1)]).unwrap_or(PLACEHOLDER)
}

/// How the graphics commands reach kitty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Direct,
    /// Through tmux's DCS passthrough.
    Tmux,
}

/// Wraps one escape sequence for tmux: every ESC doubled inside `ESC P tmux; ... ESC \`.
fn wrap(seq: &str, route: Route, out: &mut String) {
    match route {
        Route::Direct => out.push_str(seq),
        Route::Tmux => {
            out.push_str("\x1bPtmux;");
            out.push_str(&seq.replace('\x1b', "\x1b\x1b"));
            out.push_str("\x1b\\");
        }
    }
}

/// One graphics command carrying `payload`, cut into chunks of at most 4096 base64 bytes:
/// the first chunk has the keys, the others only `m` (and `q`), `m=1` on all but the last.
fn command(keys: &str, payload: &[u8], route: Route, out: &mut String) {
    let data = base64(payload);
    let chunks: Vec<&str> = if data.is_empty() {
        vec![""]
    } else {
        // Base64 is ASCII, so cutting at any byte offset is safe.
        data.as_bytes().chunks(CHUNK).map(|c| std::str::from_utf8(c).unwrap_or("")).collect()
    };
    for (k, chunk) in chunks.iter().enumerate() {
        let more = u8::from(k + 1 < chunks.len());
        let head = if k == 0 { format!("{keys},q=2,m={more}") } else { format!("q=2,m={more}") };
        wrap(&format!("\x1b_G{head};{chunk}\x1b\\"), route, out);
    }
}

/// A picture, 8-bit RGB, row by row.
pub struct Rgb8 {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// The images of one viewer. Dropping it deletes them (also on Ctrl+C, which ends the loop).
pub struct Kitty {
    route: Route,
    /// Two ids used in turn. Placeholders give the low byte in a 256-colour foreground and
    /// the high byte in a third combining mark; ids above 2^24 avoid the small ids other
    /// programs pick.
    ids: [u32; 2],
    turn: usize,
    live: Option<u32>,
}

impl Kitty {
    pub fn new(route: Route) -> Kitty {
        // Per-process ids, so that two viewers in the same window do not fight. The low byte
        // stays above 15: tmux may rewrite the 16 first colours.
        let pid = std::process::id();
        let high = 1 + pid % 255;
        let low = 16 + 2 * ((pid / 255) % 120);
        Kitty { route, ids: [high << 24 | low, high << 24 | (low + 1)], turn: 0, live: None }
    }

    /// Sends `img`, to be shown fitted into `cols` x `rows` cells, and returns its id. The
    /// placeholders must then be printed with that id, and `retire` called.
    pub fn upload(&mut self, img: &Rgb8, cols: usize, rows: usize, out: &mut String) -> u32 {
        self.turn ^= 1;
        let id = self.ids[self.turn];
        // Pictures scaled up by whole pixels repeat the row above: a good first guess for LZ77.
        let data = zlib::compress(&img.px, img.w * 3);
        let keys = format!("a=T,U=1,f=24,o=z,s={},v={},i={id},c={cols},r={rows}", img.w, img.h);
        command(&keys, &data, self.route, out);
        id
    }

    /// One row of placeholder cells: the first cell names the row, column 0 and the high byte
    /// of the id; the others follow on (kitty fills in column + 1).
    pub fn placeholder_row(id: u32, row: usize, cols: usize, out: &mut String) {
        out.push_str(&format!("\x1b[38;5;{}m", id & 0xff));
        out.push(PLACEHOLDER);
        out.push(diacritic(row));
        out.push(diacritic(0));
        out.push(diacritic((id >> 24) as usize));
        for _ in 1..cols {
            out.push(PLACEHOLDER);
        }
        out.push_str("\x1b[39m");
    }

    /// Deletes the image shown before `id`, now that the placeholders point to `id`.
    pub fn retire(&mut self, id: u32, out: &mut String) {
        if let Some(old) = self.live.replace(id) {
            if old != id {
                delete(old, self.route, out);
            }
        }
    }
}

fn delete(id: u32, route: Route, out: &mut String) {
    // `d=I`: the image, its virtual placement and its data.
    wrap(&format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"), route, out);
}

impl Drop for Kitty {
    fn drop(&mut self) {
        let mut out = String::new();
        for id in self.ids {
            delete(id, self.route, &mut out);
        }
        print!("{out}");
    }
}

/// The largest row number placeholders can name.
pub const MAX_ROWS: usize = DIACRITICS.len();

fn tmux(format: &str) -> Option<String> {
    let mut cmd = Command::new("tmux");
    cmd.arg("display-message").arg("-p");
    if let Ok(pane) = std::env::var("TMUX_PANE") {
        cmd.arg("-t").arg(pane);
    }
    let out = cmd.arg(format).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Is the terminal kitty, and how to reach it? Inside tmux, the terminal of the attached client
/// is asked for, and passthrough must be allowed; outside, `TERM` (kitty sets `xterm-kitty`)
/// or kitty's own variable tell.
pub fn detect() -> Option<Route> {
    if std::env::var_os("TMUX").is_some() {
        let info = tmux("#{client_termname} #{client_termtype}")?;
        let mut cmd = Command::new("tmux");
        cmd.args(["show-options", "-Apv"]);
        if let Ok(pane) = std::env::var("TMUX_PANE") {
            cmd.arg("-t").arg(pane);
        }
        let pass = cmd.arg("allow-passthrough").stderr(std::process::Stdio::null()).output().ok()?;
        let pass = String::from_utf8_lossy(&pass.stdout);
        let pass = pass.lines().last().unwrap_or("").trim();
        return (info.to_lowercase().contains("kitty") && (pass == "on" || pass == "all"))
            .then_some(Route::Tmux);
    }
    let term = std::env::var("TERM").unwrap_or_default();
    if term.starts_with("screen") || term.starts_with("tmux") {
        return None;
    }
    (term.contains("kitty") || std::env::var_os("KITTY_WINDOW_ID").is_some()).then_some(Route::Direct)
}

/// Size of a character cell in pixels (width, height), when the terminal says.
pub fn cell_pixels(route: Route, cols: usize, rows: usize) -> Option<(f64, f64)> {
    if route == Route::Tmux {
        let s = tmux("#{client_cell_width} #{client_cell_height}")?;
        let mut it = s.split_whitespace().filter_map(|v| v.parse::<f64>().ok());
        let (w, h) = (it.next()?, it.next()?);
        return (w > 0.0 && h > 0.0).then_some((w, h));
    }
    // SAFETY: TIOCGWINSZ fills a winsize struct.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_xpixel > 0 && ws.ws_ypixel > 0 {
            return Some((
                ws.ws_xpixel as f64 / cols.max(1) as f64,
                ws.ws_ypixel as f64 / rows.max(1) as f64,
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        // RFC 4648, section 10.
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[test]
    fn diacritics_table() {
        assert_eq!(diacritic(0), '\u{305}');
        assert_eq!(diacritic(1), '\u{30d}');
        assert_eq!(diacritic(2), '\u{30e}');
        assert_eq!(DIACRITICS[296], 0x1d244);
        assert!(DIACRITICS.windows(2).all(|w| w[0] < w[1]));
    }

    /// Splits `out` into the graphics commands it holds: (keys, payload).
    fn commands(out: &str) -> Vec<(String, String)> {
        out.split("\x1b_G")
            .skip(1)
            .map(|c| {
                let body = c.split("\x1b\\").next().unwrap();
                let (k, p) = body.split_once(';').unwrap();
                (k.to_string(), p.to_string())
            })
            .collect()
    }

    #[test]
    fn chunks_of_4096() {
        let payload: Vec<u8> = (0..10000u32).map(|i| (i * 31 % 256) as u8).collect();
        let mut out = String::new();
        command("a=T,f=24", &payload, Route::Direct, &mut out);
        let cmds = commands(&out);
        assert_eq!(cmds.len(), base64(&payload).len().div_ceil(4096));
        assert_eq!(cmds[0].0, "a=T,f=24,q=2,m=1");
        for (k, (keys, data)) in cmds.iter().enumerate() {
            let last = k + 1 == cmds.len();
            assert!(data.len() <= 4096 && (last || data.len() % 4 == 0));
            if k > 0 {
                assert_eq!(keys, if last { "q=2,m=0" } else { "q=2,m=1" });
            }
        }
        let joined: String = cmds.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(joined, base64(&payload));
    }

    #[test]
    fn small_payload_is_one_command() {
        let mut out = String::new();
        command("a=d", &[], Route::Direct, &mut out);
        assert_eq!(out, "\x1b_Ga=d,q=2,m=0;\x1b\\");
    }

    #[test]
    fn tmux_passthrough_doubles_escapes() {
        let mut out = String::new();
        wrap("\x1b_Ga=d\x1b\\", Route::Tmux, &mut out);
        assert_eq!(out, "\x1bPtmux;\x1b\x1b_Ga=d\x1b\x1b\\\x1b\\");
        // Nothing but the doubled escapes inside: no ESC \ ends the passthrough early.
        let inner = &out["\x1bPtmux;".len()..out.len() - 2];
        assert!(!inner.replace("\x1b\x1b", "").contains('\x1b'));
    }

    #[test]
    fn placeholder_grid() {
        let id = 0x03_00_00_2a;
        let mut out = String::new();
        Kitty::placeholder_row(id, 2, 3, &mut out);
        assert_eq!(out, "\x1b[38;5;42m\u{10EEEE}\u{30e}\u{305}\u{310}\u{10EEEE}\u{10EEEE}\x1b[39m");
    }

    #[test]
    fn frames_alternate_ids_and_delete_the_previous_one() {
        let mut k = Kitty::new(Route::Tmux);
        let img = Rgb8 { w: 4, h: 2, px: vec![200; 24] };
        let mut out = String::new();
        let a = k.upload(&img, 10, 5, &mut out);
        assert!(out.starts_with("\x1bPtmux;\x1b\x1b_Ga=T,U=1,f=24,o=z,s=4,v=2,i="));
        assert!(out.contains(&format!("i={a},c=10,r=5,q=2,m=0;")));
        k.retire(a, &mut out);
        assert!(!out.contains("a=d"), "nothing to delete yet");
        let b = k.upload(&img, 10, 5, &mut out);
        assert_ne!(a, b);
        assert_eq!(a >> 24, b >> 24);
        assert!((16..256).contains(&(a & 0xff)) && (16..256).contains(&(b & 0xff)));
        out.clear();
        k.retire(b, &mut out);
        assert_eq!(out, format!("\x1bPtmux;\x1b\x1b_Ga=d,d=I,i={a},q=2\x1b\x1b\\\x1b\\"));
        std::mem::forget(k); // no output from the test
    }
}
