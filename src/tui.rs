//! `laifen tui`: the brush in 3D with all its settings, live.
//!
//! The screen is drawn by the main thread; a second thread owns the Bluetooth session (or a
//! simulated brush with `--demo`) and the two talk through channels, so that the picture keeps
//! turning while a change is sent and read back.
//!
//! The picture follows the brush's state: while it brushes, the head swings about the shaft
//! with the active mode's oscillation range and speed, and shivers with its vibration
//! strength (slowed down to be seen: the real motion is much faster). The mode lights and the
//! battery light are drawn from the state too: an illustration, not a capture of the real
//! lights.

use crate::brush::{self, Brush, Options};
use crate::cmd;
use crate::decode::{self, Settings, State, HF, MODE_NAMES};
use crate::kitty;
use crate::ps2::Scene;
use crate::signals;
use crate::viz::{self, KittyView, RawTerminal, RESET};
use crate::wave::Finish;
use std::io::Write;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

pub struct TuiOptions {
    pub demo: bool,
    /// Text blocks even in kitty.
    pub blocks: bool,
    pub finish: Finish,
}

/// What the screen asks of the brush.
#[derive(Clone, Copy, Debug)]
enum Ask {
    Mode(usize),
    Tune(usize, Settings),
    Duration(u16),
    Reminder(bool),
    HighFrequency(bool),
    Power(bool),
}

impl Ask {
    /// What is being done, while it is.
    fn describe(&self) -> String {
        match self {
            Ask::Mode(i) => format!("switching to {}", MODE_NAMES[*i]),
            Ask::Tune(i, _) => format!("setting {}", MODE_NAMES[*i]),
            Ask::Duration(d) => format!("brushing time {}", clock(*d as u64)),
            Ask::Reminder(on) => format!("reminder {}", on_off(*on)),
            Ask::HighFrequency(on) => format!("high frequency {}", on_off(*on)),
            Ask::Power(true) => "starting".into(),
            Ask::Power(false) => "stopping".into(),
        }
    }

    /// What was done, once read back.
    fn done(&self) -> String {
        match self {
            Ask::Mode(i) => format!("{} active", MODE_NAMES[*i]),
            Ask::Tune(i, _) => format!("{} set", MODE_NAMES[*i]),
            Ask::Power(true) => "brushing".into(),
            Ask::Power(false) => "stopped".into(),
            other => other.describe(),
        }
    }
}

/// What the brush side reports.
enum News {
    Searching,
    Connected { firmware: String, demo: bool },
    State(State),
    Working(String),
    Done(String),
    Failed(String),
    Lost,
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

// --- The brush side -----------------------------------------------------------------------

fn apply(b: &mut Brush, ask: Ask) -> Res<State> {
    match ask {
        Ask::Mode(i) => b.set_mode(i),
        Ask::Tune(i, s) => b.tune(i, Some(s.strength), Some(s.range), Some(s.speed)).map(|(st, _)| st),
        Ask::Duration(d) => b.set_duration(d),
        Ask::Reminder(on) => b.set_reminder(on),
        Ask::HighFrequency(on) => b.set_high_frequency(on),
        Ask::Power(on) => b.set_power(on),
    }
}

fn lost(e: &str) -> bool {
    e.contains("disconnected") || e.contains("Not connected") || e.contains("interrupted")
}

/// Owns the Bluetooth session: connects (again and again, while the brush sleeps), runs what
/// the screen asks, passes on what the brush pushes, and re-reads the state now and then.
fn worker(opts: Options, asks: Receiver<Ask>, news: Sender<News>) {
    while !signals::stop_requested() {
        let _ = news.send(News::Searching);
        let mut b = match Brush::connect(&opts) {
            Ok(b) => b,
            Err(e) => {
                if !signals::stop_requested() {
                    let _ = news.send(News::Failed(e));
                    std::thread::sleep(Duration::from_secs(1));
                }
                continue;
            }
        };
        let firmware = b.version().unwrap_or_else(|_| "?".into());
        match b.state() {
            Ok(st) => {
                let _ = news.send(News::Connected { firmware, demo: false });
                let _ = news.send(News::State(st));
            }
            Err(e) => {
                let _ = news.send(News::Failed(e));
                continue;
            }
        }
        let mut last_read = Instant::now();
        let mut running = false;
        loop {
            if signals::stop_requested() {
                b.close();
                return;
            }
            match asks.recv_timeout(Duration::from_millis(30)) {
                Ok(ask) => {
                    let _ = news.send(News::Working(ask.describe()));
                    match apply(&mut b, ask) {
                        Ok(st) => {
                            running = st.running == Some(true);
                            let _ = news.send(News::State(st));
                            let _ = news.send(News::Done(ask.done()));
                        }
                        Err(e) => {
                            let gone = lost(&e);
                            let _ = news.send(News::Failed(e));
                            if gone {
                                let _ = news.send(News::Lost);
                                break;
                            }
                        }
                    }
                    last_read = Instant::now();
                }
                Err(RecvTimeoutError::Disconnected) => {
                    b.close();
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            // What the brush pushes by itself, after a press on its button for instance.
            match b.next_frame(Duration::from_millis(20)) {
                Ok(Some(Ok(f))) if f.cmd == cmd::STATE => {
                    if let Ok(st) = decode::state(&f.payload) {
                        running = st.running == Some(true);
                        let _ = news.send(News::State(st));
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = news.send(News::Failed(e));
                    let _ = news.send(News::Lost);
                    break;
                }
            }
            // Battery, and the end of a brushing session, are only known by asking.
            let every = Duration::from_secs(if running { 2 } else { 10 });
            if last_read.elapsed() > every {
                last_read = Instant::now();
                match b.state() {
                    Ok(st) => {
                        running = st.running == Some(true);
                        let _ = news.send(News::State(st));
                    }
                    Err(e) => {
                        let gone = lost(&e);
                        let _ = news.send(News::Failed(e));
                        if gone {
                            let _ = news.send(News::Lost);
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// A brush that only exists here, for trying the view without one (and for screenshots).
fn demo(asks: Receiver<Ask>, news: Sender<News>) {
    let s = |strength, range, speed| Settings { strength, range, speed };
    let mut st = State {
        mode: 1,
        modes: vec![s(3, 5, 4), s(6, 8, 6), s(9, 10, 9), s(15, 2, 2)],
        airplane: false,
        battery: 87,
        reminder: true,
        duration: 120,
        high_frequency: Some(true),
        running: Some(false),
        emergency_shutdown: None,
        charging: None,
        deep_clean: None,
    };
    let _ = news.send(News::Connected { firmware: "demo".into(), demo: true });
    let _ = news.send(News::State(st.clone()));
    let mut started: Option<Instant> = None;
    while !signals::stop_requested() {
        match asks.recv_timeout(Duration::from_millis(100)) {
            Ok(ask) => {
                let _ = news.send(News::Working(ask.describe()));
                std::thread::sleep(Duration::from_millis(250));
                let result: Res<()> = match ask {
                    Ask::Mode(i) if i == HF && st.high_frequency != Some(true) => {
                        Err("high frequency is off: turn it on first".into())
                    }
                    Ask::Mode(i) => {
                        st.mode = i as u8;
                        Ok(())
                    }
                    Ask::Tune(i, set) => {
                        st.modes[i] = set;
                        Ok(())
                    }
                    Ask::Duration(d) => {
                        st.duration = d;
                        Ok(())
                    }
                    Ask::Reminder(on) => {
                        st.reminder = on;
                        Ok(())
                    }
                    Ask::HighFrequency(false) if st.mode as usize == HF => {
                        Err("high frequency is the active mode: switch to mode 3 first".into())
                    }
                    Ask::HighFrequency(on) => {
                        st.high_frequency = Some(on);
                        Ok(())
                    }
                    Ask::Power(on) => {
                        st.running = Some(on);
                        started = on.then(Instant::now);
                        Ok(())
                    }
                };
                match result {
                    Ok(()) => {
                        let _ = news.send(News::State(st.clone()));
                        let _ = news.send(News::Done(ask.done()));
                    }
                    Err(e) => {
                        let _ = news.send(News::Failed(e));
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if started.is_some_and(|t| t.elapsed().as_secs() >= st.duration as u64) {
            started = None;
            st.running = Some(false);
            st.battery = st.battery.saturating_sub(1);
            let _ = news.send(News::State(st.clone()));
        }
    }
}

// --- The screen ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    /// The list of modes: left and right pick the mode to edit, enter switches to it.
    Modes,
    Strength,
    Range,
    Speed,
    Time,
    Reminder,
    HighFrequency,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Link {
    Searching,
    Connected,
    Demo,
    Lost,
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Busy,
    Good,
    Bad,
}

struct Ui {
    finish: Finish,
    link: Link,
    firmware: String,
    state: Option<State>,
    message: Option<(Tone, String, Instant)>,
    cursor: usize,
    /// The mode whose settings are shown and edited.
    edited: usize,
    /// A change of settings not sent yet: sent once the keys have been still for a moment.
    pending: Option<(usize, Settings, Instant)>,
    pending_time: Option<(u16, Instant)>,
    az: f64,
    el: f64,
    turntable: bool,
    /// When the current brushing session started, as seen from here.
    brushing_since: Option<Instant>,
    clock: Instant,
    frame: u64,
}

/// Colours inside a line that may be highlighted: no reset, the background stays.
const WHITE: &str = "\x1b[38;2;255;255;255m";
const EMPTY: &str = "\x1b[38;2;72;72;74m";

/// Mode names that fit the window's table.
const SHORT_NAMES: [&str; 4] = ["Mode 1", "Mode 2", "Mode 3", "High freq."];

/// How long the keys must stay still before a change of settings is sent.
const SETTLE: Duration = Duration::from_millis(450);

impl Ui {
    /// Modes that can be edited and switched to: high frequency only while it is on.
    fn modes(&self) -> Vec<usize> {
        let hf = self.state.as_ref().is_some_and(|s| s.high_frequency == Some(true) && s.modes.len() > HF);
        if hf {
            vec![0, 1, 2, HF]
        } else {
            vec![0, 1, 2]
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Modes, Row::Strength, Row::Range, Row::Speed, Row::Time, Row::Reminder];
        if self.state.as_ref().is_some_and(|s| s.high_frequency.is_some()) {
            rows.push(Row::HighFrequency);
        }
        rows
    }

    fn row(&self) -> Row {
        let rows = self.rows();
        rows[self.cursor.min(rows.len() - 1)]
    }

    /// The edited mode's settings, with a change not sent yet.
    fn shown(&self) -> Option<Settings> {
        if let Some((m, s, _)) = self.pending {
            if m == self.edited {
                return Some(s);
            }
        }
        self.state.as_ref().and_then(|st| st.modes.get(self.edited).copied())
    }

    fn running(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.running == Some(true))
    }

    fn say(&mut self, tone: Tone, text: String) {
        self.message = Some((tone, text, Instant::now()));
    }

    fn take_state(&mut self, st: State) {
        let was = self.running();
        let now = st.running == Some(true);
        if now && !was {
            self.brushing_since = Some(Instant::now());
        } else if !now {
            self.brushing_since = None;
        }
        if self.state.is_none() {
            self.edited = st.mode as usize;
        }
        self.state = Some(st);
        if !self.modes().contains(&self.edited) {
            self.edited = 2;
        }
        let n = self.rows().len();
        self.cursor = self.cursor.min(n - 1);
    }

    /// Handles one key; returns false to quit.
    fn key(&mut self, k: Key, asks: &Sender<Ask>) -> bool {
        let ask = |a: Ask| {
            let _ = asks.send(a);
        };
        match k {
            Key::Quit => return false,
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down => self.cursor = (self.cursor + 1).min(self.rows().len() - 1),
            Key::Left | Key::Right => {
                let step: i32 = if k == Key::Left { -1 } else { 1 };
                let Some(st) = self.state.clone() else { return true };
                match self.row() {
                    Row::Strength | Row::Range | Row::Speed => {
                        let Some(mut s) = self.shown() else { return true };
                        let (lo, hi) = match self.row() {
                            Row::Strength => brush::strength_range(self.edited),
                            _ => (1, 10),
                        };
                        let v = match self.row() {
                            Row::Strength => &mut s.strength,
                            Row::Range => &mut s.range,
                            _ => &mut s.speed,
                        };
                        *v = (*v as i32 + step).clamp(lo as i32, hi as i32) as u8;
                        self.pending = Some((self.edited, s, Instant::now()));
                    }
                    Row::Time => {
                        let now = self.pending_time.map(|(d, _)| d).unwrap_or(st.duration);
                        let d = (now as i32 + 30 * step).clamp(60, 300) as u16;
                        self.pending_time = Some((d, Instant::now()));
                    }
                    Row::Reminder => ask(Ask::Reminder(!st.reminder)),
                    Row::HighFrequency => ask(Ask::HighFrequency(st.high_frequency != Some(true))),
                    Row::Modes => {
                        let modes = self.modes();
                        let at = modes.iter().position(|&m| m == self.edited).unwrap_or(0) as i32;
                        self.edited = modes[(at + step).rem_euclid(modes.len() as i32) as usize];
                    }
                }
            }
            Key::Enter => {
                if self.row() == Row::Modes {
                    ask(Ask::Mode(self.edited));
                }
            }
            Key::Digit(d) => {
                let m = d as usize - 1;
                if self.modes().contains(&m) {
                    self.edited = m;
                    ask(Ask::Mode(m));
                }
            }
            Key::Space => ask(Ask::Power(!self.running())),
            Key::Char('c') => {
                self.finish = self.finish.next();
                self.say(Tone::Good, format!("finish: {}", self.finish.name()));
            }
            Key::Char('o') => self.turntable = !self.turntable,
            Key::Char('[') => self.az -= 15.0,
            Key::Char(']') => self.az += 15.0,
            Key::Char('0') => {
                self.az = 20.0;
                self.el = 10.0;
            }
            Key::Char(_) => {}
        }
        true
    }

    /// Sends the settings once the keys have been still long enough.
    fn flush(&mut self, asks: &Sender<Ask>) {
        if let Some((m, s, t)) = self.pending {
            if t.elapsed() >= SETTLE {
                self.pending = None;
                if self.state.as_ref().and_then(|st| st.modes.get(m)) != Some(&s) {
                    let _ = asks.send(Ask::Tune(m, s));
                }
            }
        }
        if let Some((d, t)) = self.pending_time {
            if t.elapsed() >= SETTLE {
                self.pending_time = None;
                if self.state.as_ref().map(|st| st.duration) != Some(d) {
                    let _ = asks.send(Ask::Duration(d));
                }
            }
        }
    }

    fn scene(&self) -> Scene {
        let t = self.clock.elapsed().as_secs_f64();
        let mut scene = Scene { finish: self.finish, ..Scene::default() };
        let Some(st) = &self.state else { return scene };
        let awake = matches!(self.link, Link::Connected | Link::Demo);
        if awake {
            let m = st.mode as usize;
            scene.leds = if m == HF { [true; 3] } else { [m == 0, m == 1, m == 2] };
            scene.battery = Some(match st.battery {
                50.. => [0.35, 0.95, 0.45],
                20..=49 => [1.0, 0.72, 0.2],
                _ => [1.0, 0.25, 0.2],
            });
        }
        if self.running() {
            if let Some(s) = st.modes.get(st.mode as usize) {
                // Slowed down so that it can be seen: the real head is far quicker.
                let amplitude = (s.range as f64 / 10.0) * 30f64.to_radians();
                let freq = 0.8 + 0.22 * s.speed as f64;
                scene.swing = amplitude * (std::f64::consts::TAU * freq * t).sin();
                let (lo, hi) = brush::strength_range(st.mode as usize);
                let k = (s.strength.saturating_sub(lo) as f64 + 1.0) / (hi - lo + 1) as f64;
                let jitter = |seed: u64| {
                    let x = (self.frame.wrapping_mul(6364136223846793005).wrapping_add(seed) >> 33) as f64;
                    (x / (1u64 << 31) as f64) * 2.0 - 1.0
                };
                scene.tilt = [k * 0.9f64.to_radians() * jitter(1), k * 0.9f64.to_radians() * jitter(7)];
            }
        }
        scene
    }

    /// The window's lines (32 visible characters at most).
    fn window(&self) -> Vec<String> {
        // The vendor's app uses iOS's system colours: grey labels, blue accent, green, orange
        // and red for states. Their dark-mode values, on the window's #1c1c1e.
        let dim = "\x1b[38;2;142;142;147m";
        let title = "\x1b[1m\x1b[38;2;255;255;255m";
        let hi = "\x1b[38;2;255;255;255m\x1b[48;2;10;132;255m";
        let green = "\x1b[38;2;48;209;88m";
        let yellow = "\x1b[38;2;255;159;10m";
        let red = "\x1b[38;2;255;69;58m";
        let blue = "\x1b[38;2;10;132;255m";
        let mut out = Vec::new();
        let link = match self.link {
            Link::Searching => format!("{yellow}○ looking for it{RESET}"),
            Link::Connected => format!("{blue}● connected{RESET}"),
            Link::Demo => format!("{blue}● demo{RESET}"),
            Link::Lost => format!("{red}○ asleep{RESET}"),
        };
        out.push(format!("{title}Laifen Wave{RESET}{}{link}", " ".repeat(32 - 11 - viz::visible_len(&link))));
        let Some(st) = &self.state else {
            out.push(String::new());
            out.push("Press the brush's button to wake".into());
            out.push("it. A brush connected to the".into());
            out.push("Laifen app on a phone stays".into());
            out.push("invisible: close the app.".into());
            out.push(String::new());
            out.push(format!("{dim}q quit{RESET}"));
            return out;
        };
        let level = st.battery.min(100) as usize;
        let colour = match st.battery {
            50.. => green,
            20..=49 => yellow,
            _ => red,
        };
        let full = level.div_ceil(10).min(10);
        let battery =
            format!("{colour}{}{EMPTY}{}{RESET} {level:>3}%", "█".repeat(full), "█".repeat(10 - full));
        let fw = format!("{dim}{}{RESET}", self.firmware);
        out.push(format!(
            "{fw}{}{battery}",
            " ".repeat(32usize.saturating_sub(viz::visible_len(&fw) + viz::visible_len(&battery)))
        ));
        out.push(String::new());
        out.push(format!("{dim}  {:<12} {:>5} {:>5} {:>5}{RESET}", "mode", "vibr.", "range", "speed"));
        let row = self.row();
        for r in self.modes() {
            let s = if Some(r) == self.pending.map(|p| p.0) { self.shown().unwrap() } else { st.modes[r] };
            let active = if st.mode as usize == r { format!("{blue}●{RESET}") } else { " ".into() };
            let text = format!("{:<12} {:>5} {:>5} {:>5}", SHORT_NAMES[r], s.strength, s.range, s.speed);
            out.push(match (r == self.edited, row == Row::Modes) {
                (true, true) => format!("{active} {hi}{text}{RESET}"),
                (true, false) => format!("{active} \x1b[38;2;255;255;255m{text}{RESET}"),
                _ => format!("{active} {dim}{text}{RESET}"),
            });
        }
        out.push(String::new());
        out.push(format!("{title}{}{RESET}", MODE_NAMES[self.edited]));
        let shown = self.shown().unwrap_or(st.modes[0]);
        let (lo, hi_s) = brush::strength_range(self.edited);
        for (r, label, v, lo) in [
            (Row::Strength, "strength", shown.strength, lo),
            (Row::Range, "range", shown.range, 1),
            (Row::Speed, "speed", shown.speed, 1),
        ] {
            let _ = hi_s;
            // White for the level, dark grey for the rest, as the app's sliders; only colour
            // changes inside, so that a selected row keeps its background to the end.
            let filled = (v.saturating_sub(lo) as usize + 1).min(10);
            let gauge = format!("{WHITE}{}{EMPTY}{}{WHITE}", "█".repeat(filled), "█".repeat(10 - filled));
            let text = format!("{label:<9} {gauge} {v:>3}");
            out.push(if row == r {
                format!("  {hi}{text}{RESET}")
            } else {
                format!("  {WHITE}{text}{RESET}")
            });
        }
        out.push(String::new());
        let time = self.pending_time.map(|(d, _)| d).unwrap_or(st.duration);
        let mut setting = |r: Row, label: &str, value: String| {
            let text = format!("{label:<14}{value}");
            out.push(if row == r { format!("  {hi}{text}{RESET}") } else { format!("  {text}") });
        };
        setting(Row::Time, "brushing time", clock(time as u64));
        setting(Row::Reminder, "30 s reminder", on_off(st.reminder).into());
        if let Some(hf) = st.high_frequency {
            setting(Row::HighFrequency, "high freq.", on_off(hf).into());
        }
        if st.airplane {
            out.push(format!("  {yellow}airplane mode: changes refused{RESET}"));
        }
        out.push(String::new());
        if self.running() {
            let secs = self.brushing_since.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            let total = st.duration as u64;
            let parts = (total / 30).max(1);
            let done = (secs / 30).min(parts);
            let segments: String = (0..parts)
                .map(|i| {
                    if i < done {
                        '▰'
                    } else if i == done {
                        '▶'
                    } else {
                        '▱'
                    }
                })
                .collect();
            out.push(format!("{green}⏵ brushing {} / {}{RESET} {segments}", clock(secs), clock(total)));
        } else {
            out.push(format!("{dim}  not brushing{RESET}"));
        }
        match &self.message {
            Some((tone, text, at)) if at.elapsed() < Duration::from_secs(6) || *tone == Tone::Busy => {
                let (mark, colour) = match tone {
                    Tone::Busy => ("…", yellow),
                    Tone::Good => ("✓", green),
                    Tone::Bad => ("✗", red),
                };
                let text: String = text.chars().take(30).collect();
                out.push(format!("{colour}{mark} {text}{RESET}"));
            }
            _ => out.push(String::new()),
        }
        out.push(String::new());
        out.push(format!("{dim}↑↓ move  ←→ change  enter use{RESET}"));
        out.push(format!("{dim}space brush  1-4 mode  c finish{RESET}"));
        out.push(format!("{dim}o turn  [ ] view  q quit{RESET}"));
        out
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Space,
    Quit,
    Digit(u8),
    Char(char),
}

/// Key presses in a chunk of terminal input (arrows arrive as ESC [ A..D, or ESC O A..D).
fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            match (bytes.get(i + 1), bytes.get(i + 2)) {
                (Some(b'[') | Some(b'O'), Some(c)) => {
                    match c {
                        b'A' => out.push(Key::Up),
                        b'B' => out.push(Key::Down),
                        b'C' => out.push(Key::Right),
                        b'D' => out.push(Key::Left),
                        _ => {}
                    }
                    i += 3;
                    continue;
                }
                (None, _) => out.push(Key::Quit),
                _ => {}
            }
            i += 1;
            continue;
        }
        out.push(match b {
            b'\r' | b'\n' => Key::Enter,
            b' ' => Key::Space,
            b'q' => Key::Quit,
            b'k' => Key::Up,
            b'j' => Key::Down,
            b'h' => Key::Left,
            b'l' => Key::Right,
            b'1'..=b'4' => Key::Digit(b - b'0'),
            c => Key::Char(c as char),
        });
        i += 1;
    }
    out
}

pub fn run(tui: TuiOptions, opts: Options) -> Res<()> {
    let (ask_tx, ask_rx) = mpsc::channel();
    let (news_tx, news_rx) = mpsc::channel();
    let worker = if tui.demo {
        std::thread::spawn(move || demo(ask_rx, news_tx))
    } else {
        std::thread::spawn(move || worker(opts, ask_rx, news_tx))
    };
    let route = if tui.blocks { None } else { kitty::detect() };
    let mut view = route.map(KittyView::new);
    let mut ui = Ui {
        finish: tui.finish,
        link: Link::Searching,
        firmware: String::new(),
        state: None,
        message: None,
        cursor: 0,
        edited: 0,
        pending: None,
        pending_time: None,
        az: 20.0,
        el: 10.0,
        turntable: true,
        brushing_since: None,
        clock: Instant::now(),
        frame: 0,
    };
    let term = RawTerminal::enter();
    let mut last = Instant::now();
    let mut dirty = true;
    let mut out = std::io::stdout();
    'run: while !signals::stop_requested() {
        for k in keys(&term.keys()) {
            if !ui.key(k, &ask_tx) {
                break 'run;
            }
            dirty = true;
        }
        ui.flush(&ask_tx);
        loop {
            match news_rx.try_recv() {
                Ok(n) => {
                    dirty = true;
                    match n {
                        News::Searching => {
                            ui.link = if ui.link == Link::Lost { Link::Lost } else { Link::Searching }
                        }
                        News::Connected { firmware, demo } => {
                            ui.link = if demo { Link::Demo } else { Link::Connected };
                            ui.firmware = firmware;
                        }
                        News::State(st) => ui.take_state(st),
                        News::Working(w) => ui.say(Tone::Busy, w),
                        News::Done(w) => ui.say(Tone::Good, w),
                        News::Failed(e) => ui.say(Tone::Bad, e),
                        News::Lost => ui.link = Link::Lost,
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'run,
            }
        }
        let animated = ui.turntable || ui.running();
        let dt = last.elapsed().as_secs_f64();
        if ui.turntable {
            ui.az = (ui.az + 14.0 * dt) % 360.0;
        }
        last = Instant::now();
        if animated || dirty || ui.brushing_since.is_some() {
            ui.frame += 1;
            let (cols, rows) = viz::terminal_size();
            let window = ui.window();
            let scene = ui.scene();
            let frame = match view.as_mut() {
                Some(v) => v.frame(&scene, cols, rows, ui.az, ui.el, &window),
                None => viz::blocks(&scene, cols, rows, ui.az, ui.el, &window),
            };
            let _ = out.write_all(frame.as_bytes());
            let _ = out.flush();
            dirty = false;
        }
        let pace = if animated { 70 } else { 120 };
        std::thread::sleep(Duration::from_millis(pace).saturating_sub(last.elapsed()));
    }
    // Leave the brush disconnected and the terminal as it was.
    signals::request_stop();
    drop(ask_tx);
    let deadline = Instant::now() + Duration::from_secs(4);
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(view);
    drop(term);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_from_terminal_input() {
        assert_eq!(keys(b"\x1b[A\x1b[Bjk"), vec![Key::Up, Key::Down, Key::Down, Key::Up]);
        assert_eq!(keys(b"\x1bOC h"), vec![Key::Right, Key::Space, Key::Left]);
        assert_eq!(keys(b"\x1b"), vec![Key::Quit]);
        assert_eq!(keys(b"2\rq"), vec![Key::Digit(2), Key::Enter, Key::Quit]);
    }

    fn ui_with_state() -> Ui {
        let s = |strength, range, speed| Settings { strength, range, speed };
        let st = State {
            mode: 1,
            modes: vec![s(1, 10, 7), s(10, 10, 10), s(4, 10, 10), s(20, 1, 1)],
            airplane: false,
            battery: 91,
            reminder: true,
            duration: 120,
            high_frequency: Some(true),
            running: Some(false),
            emergency_shutdown: None,
            charging: None,
            deep_clean: None,
        };
        let mut ui = Ui {
            finish: Finish::Steel,
            link: Link::Connected,
            firmware: "V2.0.8.Y44.303".into(),
            state: None,
            message: None,
            cursor: 0,
            edited: 0,
            pending: None,
            pending_time: None,
            az: 0.0,
            el: 0.0,
            turntable: false,
            brushing_since: None,
            clock: Instant::now(),
            frame: 0,
        };
        ui.take_state(st);
        ui
    }

    #[test]
    fn the_window_fits_and_shows_the_state() {
        let ui = ui_with_state();
        let w = ui.window();
        for line in &w {
            assert!(viz::visible_len(line) <= 32, "{line:?} is {} wide", viz::visible_len(line));
        }
        let all = w.join("\n");
        assert!(all.contains("Mode 2") && all.contains("91%") && all.contains("2:00"));
    }

    #[test]
    fn arrows_edit_the_selected_mode_and_send_after_a_pause() {
        let mut ui = ui_with_state();
        let (tx, rx) = mpsc::channel();
        // The cursor starts on the modes, the active one (Mode 2) picked; down to strength,
        // then lower it.
        assert_eq!((ui.row(), ui.edited), (Row::Modes, 1));
        ui.key(Key::Down, &tx);
        assert_eq!(ui.row(), Row::Strength);
        ui.key(Key::Left, &tx);
        ui.key(Key::Left, &tx);
        assert_eq!(ui.shown().unwrap().strength, 8);
        ui.flush(&tx);
        assert!(rx.try_recv().is_err(), "sent before the keys settled");
        ui.pending = ui.pending.map(|(m, s, _)| (m, s, Instant::now() - SETTLE));
        ui.flush(&tx);
        match rx.try_recv() {
            Ok(Ask::Tune(1, s)) => assert_eq!(s.strength, 8),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn left_and_right_pick_a_mode_and_enter_switches_to_it() {
        let mut ui = ui_with_state();
        let (tx, rx) = mpsc::channel();
        ui.key(Key::Right, &tx);
        ui.key(Key::Right, &tx);
        assert_eq!(ui.edited, HF);
        ui.key(Key::Right, &tx);
        assert_eq!(ui.edited, 0, "wraps round");
        ui.key(Key::Enter, &tx);
        assert!(matches!(rx.try_recv(), Ok(Ask::Mode(0))));
    }

    /// The window of `laifen tui` for the README, from the state read on the owner's brush at
    /// 17:34 on 2026-10-16 (`ui_with_state`, battery 90 %), to $LAIFEN_PREVIEW_DIR/windows:
    /// idle (`tui.ansi`), then brushing, 10 frames a second for 5 s (`brushing/`).
    #[test]
    #[ignore]
    fn readme_windows() {
        let dir = std::env::var("LAIFEN_PREVIEW_DIR").expect("LAIFEN_PREVIEW_DIR");
        std::fs::create_dir_all(format!("{dir}/windows/brushing")).unwrap();
        let mut ui = ui_with_state();
        let mut st = ui.state.clone().unwrap();
        st.battery = 90;
        ui.take_state(st.clone());
        std::fs::write(format!("{dir}/windows/tui.ansi"), viz::window_only(150, 42, &ui.window())).unwrap();
        st.running = Some(true);
        ui.take_state(st);
        ui.say(Tone::Good, "brushing".into());
        for i in 0..50u64 {
            ui.brushing_since = Some(Instant::now() - Duration::from_millis(100 * i + 50));
            ui.message = Some((Tone::Good, "brushing".into(), Instant::now()));
            let screen = viz::window_only(150, 42, &ui.window());
            std::fs::write(format!("{dir}/windows/brushing/frame-{i:03}.ansi"), screen).unwrap();
        }
    }

    #[test]
    fn brushing_moves_the_head() {
        let mut ui = ui_with_state();
        let mut st = ui.state.clone().unwrap();
        st.running = Some(true);
        ui.take_state(st);
        ui.clock = Instant::now() - Duration::from_millis(130);
        let s = ui.scene();
        assert!(s.swing.abs() > 0.0 && s.leds == [false, true, false] && s.battery.is_some());
    }
}
