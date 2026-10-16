//! Putting the 3D brush on the terminal, with a window of text floating over it. In kitty the
//! picture is a real image (`KittyView`); elsewhere it is drawn at two pixels per character
//! cell, as `▀` half blocks in 24-bit colour. Adapted from nothing-toolbox's head-tracking
//! view (same author).

use crate::kitty::{self, Kitty, Route};
use crate::ps2::{self, Scene};

pub const RESET: &str = "\x1b[0m";

/// Width of the window, flush right, over the picture.
pub const WINDOW: usize = 36;
/// Its background, a dark slate.
const WINDOW_BG: &str = "\x1b[48;2;14;16;22m";

/// Prints row `row` of a picture as half blocks: the upper pixel in the foreground of `▀`,
/// the lower one in its background.
fn encode(img: &kitty::Rgb8, row: usize, width: usize, out: &mut String) {
    let (mut fg, mut bg): (Option<[u8; 3]>, Option<[u8; 3]>) = (None, None);
    out.push_str(RESET);
    let px = |x: usize, y: usize| -> Option<[u8; 3]> {
        (y < img.h && x < img.w).then(|| {
            let p = &img.px[(y * img.w + x) * 3..][..3];
            [p[0], p[1], p[2]]
        })
    };
    for x in 0..width.min(img.w) {
        let (top, bottom) = (px(x, 2 * row), px(x, 2 * row + 1));
        let (ch, want_fg, want_bg) = match (top, bottom) {
            (None, None) => (' ', fg, None),
            (Some(t), b) => ('▀', Some(t), b),
            (None, Some(b)) => ('▄', Some(b), None),
        };
        if want_bg != bg {
            match want_bg {
                Some([r, g, b]) => out.push_str(&format!("\x1b[48;2;{r};{g};{b}m")),
                None => out.push_str("\x1b[49m"),
            }
            bg = want_bg;
        }
        if want_fg != fg {
            if let Some([r, g, b]) = want_fg {
                out.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
            }
            fg = want_fg;
        }
        out.push(ch);
    }
    out.push_str(RESET);
}

/// Length of `s` on screen: the text without its colour sequences.
pub fn visible_len(s: &str) -> usize {
    let (mut n, mut esc) = (0, false);
    for c in s.chars() {
        match (esc, c) {
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, '\x1b') => esc = true,
            (false, _) => n += 1,
        }
    }
    n
}

/// The window's text framed in a box, every line `WINDOW` cells wide on its dark background.
fn framed(window: &[String]) -> Vec<String> {
    let frame = "\x1b[38;5;245m";
    let bar = "─".repeat(WINDOW - 2);
    let mut lines = vec![format!("{WINDOW_BG}{frame}┌{bar}┐{RESET}")];
    for line in window {
        // The colour sequences end with a reset: bring the background back after each.
        let line = line.replace(RESET, &format!("{RESET}{WINDOW_BG}"));
        let pad = (WINDOW - 4).saturating_sub(visible_len(&line));
        lines.push(format!(
            "{WINDOW_BG}{frame}│{RESET}{WINDOW_BG} {line}{}{frame}│{RESET}",
            " ".repeat(pad + 1)
        ));
    }
    lines.push(format!("{WINDOW_BG}{frame}└{bar}┘{RESET}"));
    lines
}

/// The screen of `laifen tui` without its picture: the window alone, flush right. For the
/// README's images, which draw the picture underneath.
#[cfg(test)]
pub fn window_only(cols: usize, rows: usize, window: &[String]) -> String {
    let mut out = String::new();
    compose(rows, cols, window, &mut out, |_, width, out| out.push_str(&" ".repeat(width)));
    out
}

/// `h` lines over a picture `cols` wide (drawn by `row(y, width, out)`): the window sits flush
/// right, one line down, in front of the picture.
fn compose(
    h: usize,
    cols: usize,
    window: &[String],
    out: &mut String,
    mut row: impl FnMut(usize, usize, &mut String),
) {
    let lines = framed(window);
    for y in 0..h {
        match y.checked_sub(1).and_then(|i| lines.get(i)).filter(|_| cols > WINDOW + 8) {
            Some(line) => {
                row(y, cols - WINDOW, out);
                out.push_str(line);
            }
            None => row(y, cols, out),
        }
        // The last line has no newline: a full screen would scroll.
        out.push_str(if y + 1 < h { "\x1b[K\n" } else { "\x1b[K" });
    }
}

/// The picture's horizontal place: the brush is drawn left of the window.
fn picture_offset(cols: usize) -> f64 {
    if cols > WINDOW + 8 {
        -(WINDOW as f64) / 2.0 / cols as f64
    } else {
        0.0
    }
}

/// A full frame in text blocks (two pixels per cell), for terminals without kitty graphics.
pub fn blocks(scene: &Scene, cols: usize, rows: usize, az: f64, el: f64, window: &[String]) -> String {
    let (w, h) = (cols.max(20), rows.max(10));
    let img = ps2::render_shifted(scene, w, 2 * h, az, el, picture_offset(w));
    let mut out = String::from("\x1b[H");
    compose(h, w, window, &mut out, |y, width, out| encode(&img, y, width, out));
    out
}

/// Pixels per cell when the terminal does not say (a common kitty font size).
const CELL: (f64, f64) = (10.0, 21.0);
/// Lines of the rendered picture, before it is scaled up to the cells: PS2 territory.
const LINES: f64 = 200.0;

/// The same view as `blocks`, drawn as a real picture through the kitty graphics protocol.
pub struct KittyView {
    kitty: Kitty,
    route: Route,
    /// Cell size in pixels, for the terminal size it was asked for.
    cell: Option<((usize, usize), (f64, f64))>,
}

impl KittyView {
    pub fn new(route: Route) -> KittyView {
        KittyView { kitty: Kitty::new(route), route, cell: None }
    }

    pub fn frame(
        &mut self,
        scene: &Scene,
        cols: usize,
        rows: usize,
        az: f64,
        el: f64,
        window: &[String],
    ) -> String {
        let w = cols.max(20);
        let h = rows.clamp(10, kitty::MAX_ROWS);
        let (cw, ch) = match self.cell {
            Some((size, cell)) if size == (cols, rows) => cell,
            _ => {
                let cell = kitty::cell_pixels(self.route, cols, rows).unwrap_or(CELL);
                self.cell = Some(((cols, rows), cell));
                cell
            }
        };
        // Render small, then scale up by a whole factor to about the size of the cells: the
        // terminal only has a few percent left to stretch.
        let (bw, bh) = (w as f64 * cw, h as f64 * ch);
        let k = (bh / LINES).round().max(1.0);
        let (pw, ph) = (((bw / k) as usize).max(16), ((bh / k) as usize).max(16));
        let img = ps2::render_shifted(scene, pw, ph, az, el, picture_offset(w));
        let picture = ps2::upscale(&img, k as usize);

        let mut out = String::new();
        let id = self.kitty.upload(&picture, w, h, &mut out);
        out.push_str("\x1b[H");
        compose(h, w, window, &mut out, |y, width, out| {
            Kitty::placeholder_row(id, y, width, out);
            out.push_str(RESET);
        });
        self.kitty.retire(id, &mut out);
        out
    }
}

/// Raw keyboard input (cbreak, no echo) on the alternate screen, restored on drop.
pub struct RawTerminal {
    saved: Option<libc::termios>,
}

impl RawTerminal {
    pub fn enter() -> RawTerminal {
        // SAFETY: termios calls on stdin with a properly sized struct.
        let saved = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::isatty(0) == 0 || libc::tcgetattr(0, &mut t) != 0 {
                None
            } else {
                let saved = t;
                t.c_lflag &= !(libc::ICANON | libc::ECHO);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(0, libc::TCSANOW, &t);
                Some(saved)
            }
        };
        print!("\x1b[?1049h\x1b[2J\x1b[?25l");
        RawTerminal { saved }
    }

    /// Non-blocking read of pending key bytes.
    pub fn keys(&self) -> Vec<u8> {
        if self.saved.is_none() {
            return Vec::new();
        }
        let mut buf = [0u8; 64];
        // SAFETY: reading into a local buffer of the given size; VMIN=0 makes it non-blocking.
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            buf[..n as usize].to_vec()
        } else {
            Vec::new()
        }
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Some(t) = self.saved {
            // SAFETY: restoring the attributes saved in `enter`.
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &t);
            }
        }
        print!("\x1b[?25h\x1b[2J\x1b[?1049l");
    }
}

pub fn terminal_size() -> (usize, usize) {
    // SAFETY: TIOCGWINSZ fills a winsize struct.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
    }
    (100, 40)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_blocks_fill_the_terminal_with_the_window_on_the_right() {
        let window = vec!["Laifen Wave".to_string(), "\x1b[96mMode 2\x1b[0m".to_string()];
        let f = blocks(&Scene::default(), 104, 28, 20.0, 10.0, &window);
        let lines: Vec<&str> = f.trim_start_matches("\x1b[H").split('\n').collect();
        assert_eq!(lines.len(), 28);
        for line in &lines {
            assert_eq!(visible_len(&line.replace("\x1b[K", "")), 104, "{line:?}");
        }
        assert!(f.contains("Laifen Wave") && f.contains('▀'));
    }

    /// Time and size of kitty frames for a common terminal (200 x 50 cells of 10 x 21 px),
    /// the brush turning: `cargo test --release kitty_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn kitty_cost() {
        let mut view = KittyView::new(Route::Direct);
        view.cell = Some(((200, 50), (10.0, 21.0)));
        let start = std::time::Instant::now();
        let mut bytes = 0;
        for i in 0..20 {
            let scene = Scene { swing: (i as f64 * 0.4).sin() * 0.5, ..Scene::default() };
            bytes += view.frame(&scene, 200, 50, i as f64 * 3.0, 10.0, &[]).len();
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!("{ms:.1} ms and {} kB per frame", bytes / 20 / 1000);
        std::mem::forget(view);
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let _ = blocks(&Scene::default(), 5, 3, 0.0, 0.0, &[]);
    }
}
