//! `laifen`: Linux control of the Laifen Wave electric toothbrush (model LFTB01-P) over
//! Bluetooth LE, through BlueZ. Other Laifen brushes speak another protocol: not supported.

mod bluez;
mod brush;
mod cmd;
mod dbus;
mod decode;
mod frame;
mod kitty;
mod ps2;
mod signals;
mod tui;
mod viz;
mod wave;
mod zlib;

use brush::{Brush, Options};
use clap::{Args, CommandFactory, Parser, Subcommand};
use decode::{Settings, State, HF, MODE_ARGS, MODE_NAMES};
use std::process::ExitCode;
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

const EXAMPLES: &str = "\
Examples:
  laifen status                          everything at a glance
  laifen mode set 2
  laifen mode tune 1 --strength 3 --speed 5
  laifen duration set 2m30
  laifen reminder off
  laifen start                           brush with the active mode; laifen stop stops it
  laifen watch                           what the brush reports, until Ctrl+C
  laifen completions zsh > ~/.zfunc/_laifen

Wake the brush first (press its button), and close the Laifen app on your phone: a brush
connected to the phone is most likely invisible to the computer.
Protocol notes: https://github.com/marc-alexis-com/laifen-toolbox/blob/main/docs/PROTOCOL.md";

#[derive(Parser)]
#[command(
    name = "laifen",
    version,
    about = "Control a Laifen Wave toothbrush (LFTB01-P) from Linux",
    long_about = "Control a Laifen Wave electric toothbrush (model LFTB01-P) from Linux, over \
                  Bluetooth LE, through BlueZ.\n\nEach command finds the brush, connects, does its \
                  work and disconnects. The brush only shows up while it is awake: press its button \
                  first.\n\nOnly commands tested on a real brush can be sent. Factory reset, \
                  calibration, airplane mode, emergency shutdown and firmware updates are not \
                  implemented.",
    after_long_help = EXAMPLES
)]
struct Cli {
    /// Brush address (default: $LAIFEN_MAC, else the first Laifen Wave that shows up)
    #[arg(long, global = true, value_name = "ADDRESS", help_heading = "Global options")]
    mac: Option<String>,
    /// How long to wait for the brush to show up, in seconds
    #[arg(
        long,
        global = true,
        default_value_t = 30,
        value_name = "SECONDS",
        help_heading = "Global options"
    )]
    wait: u64,
    /// How long to wait for each answer, in milliseconds
    #[arg(long, global = true, default_value_t = 3000, value_name = "MS", help_heading = "Global options")]
    timeout: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Everything at a glance: firmware, battery, modes and their settings
    Status {
        /// Print one JSON object instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Battery level
    Battery {
        /// Print {"level":N}
        #[arg(long)]
        json: bool,
    },
    /// The active mode, switching modes, and each mode's settings
    #[command(after_long_help = "Examples:\n  laifen mode get\n  laifen mode set hf\n  \
                                 laifen mode tune 2 --strength 6\n  laifen mode tune hf --strength 15")]
    Mode {
        #[command(subcommand)]
        action: ModeAction,
    },
    /// Brushing time, the same for every mode
    Duration {
        #[command(subcommand)]
        action: DurationAction,
    },
    /// The short vibration every 30 seconds, to move on to the next part of the mouth
    Reminder(SwitchArgs),
    /// High frequency, the brush's fourth mode
    #[command(name = "high-freq", visible_alias = "hf")]
    HighFreq(SwitchArgs),
    /// Start brushing, with the active mode
    Start,
    /// Stop brushing
    Stop,
    /// Print what the brush reports (state changes) until Ctrl+C or until it disconnects
    Watch {
        /// Print the frames in hex
        #[arg(long)]
        raw: bool,
    },
    /// List the Laifen devices advertising nearby
    Scan {
        /// How long to listen
        #[arg(long, default_value_t = 10, value_name = "SECONDS")]
        seconds: u64,
    },
    /// The brush in 3D with all its settings, live: change them with the arrow keys
    #[command(after_long_help = "In kitty (tmux included, with allow-passthrough on) the brush is a real \
                                 picture; elsewhere it is drawn in text blocks. While the brush runs, its \
                                 head swings with the active mode's range and speed.\n\nKeys: up/down \
                                 move, left/right change, enter switch to the mode under the cursor, 1-4 \
                                 switch mode, space start/stop brushing, c finish, o turntable, [ ] turn \
                                 the view, q quit.")]
    Tui {
        /// A simulated brush, to try the view without one
        #[arg(long)]
        demo: bool,
        /// Text blocks even in kitty
        #[arg(long)]
        blocks: bool,
        /// The handle's finish
        #[arg(long, value_enum, default_value_t = FinishArg::Steel)]
        finish: FinishArg,
    },
    /// One connection, several commands, one per line: typed, or piped in
    #[command(after_long_help = "Every command of laifen works inside, except shell, scan, watch and \
                                 completions; plus `sleep SECONDS` and `quit`. If the brush goes to \
                                 sleep in between, the next command reconnects.\n\nExamples:\n  \
                                 laifen shell\n  printf 'duration set 2m30\\nreminder off\\n' | laifen shell")]
    Shell,
    /// Print a shell completion script
    Completions { shell: clap_complete::Shell },
}

#[derive(Subcommand)]
enum ModeAction {
    /// The active mode and its settings
    Get,
    /// Switch to another mode: 1, 2, 3 or hf
    Set {
        /// 1, 2, 3 or hf
        #[arg(value_parser = mode_arg, value_name = "MODE")]
        mode: usize,
    },
    /// Change a mode's settings, without switching to it
    Tune {
        /// 1, 2, 3 or hf
        #[arg(value_parser = mode_arg, value_name = "MODE")]
        mode: usize,
        /// Vibration strength, 1-10 (11-20 for hf)
        #[arg(long, value_name = "N")]
        strength: Option<u8>,
        /// Oscillation range, 1-10
        #[arg(long, value_name = "N")]
        range: Option<u8>,
        /// Oscillation speed, 1-10
        #[arg(long, value_name = "N")]
        speed: Option<u8>,
    },
}

#[derive(Subcommand)]
enum DurationAction {
    /// The brushing time
    Get,
    /// 1 to 5 minutes in 30-second steps: 150, 150s, 2m30, 2:30, 3m
    Set {
        #[arg(value_parser = duration_arg, value_name = "DURATION")]
        duration: u16,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum FinishArg {
    Steel,
    Aluminium,
    White,
}

#[derive(Args)]
struct SwitchArgs {
    #[command(subcommand)]
    action: Switch,
}

#[derive(Subcommand, Clone, Copy)]
enum Switch {
    /// Print on or off
    Get,
    On,
    Off,
}

fn mode_arg(s: &str) -> Res<usize> {
    MODE_ARGS.iter().position(|m| m.eq_ignore_ascii_case(s)).ok_or_else(|| "1, 2, 3 or hf".to_string())
}

/// Seconds from 150, 150s, 2m30, 2m30s, 2m, 2:30. The vendor app offers 1 to 5 minutes in
/// 30-second steps; nothing else is sent.
fn duration_arg(s: &str) -> Res<u16> {
    let s = s.trim().to_ascii_lowercase();
    let num = |t: &str| t.parse::<u32>().map_err(|_| format!("not a duration: {s}"));
    let secs = if let Some((m, rest)) = s.split_once(['m', ':']) {
        let rest = rest.trim_end_matches('s');
        num(m)? * 60 + if rest.is_empty() { 0 } else { num(rest)? }
    } else {
        num(s.trim_end_matches('s'))?
    };
    if !(60..=300).contains(&secs) || !secs.is_multiple_of(30) {
        return Err(format!("{secs} s: 1 to 5 minutes in 30-second steps, as the vendor app offers"));
    }
    Ok(secs as u16)
}

fn switch_name(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn duration_text(secs: u16) -> String {
    match secs % 60 {
        0 => format!("{} min", secs / 60),
        r => format!("{} min {r} s", secs / 60),
    }
}

fn settings_text(s: &Settings) -> String {
    format!("strength {}, range {}, speed {}", s.strength, s.range, s.speed)
}

fn mode_name(i: u8) -> String {
    MODE_NAMES.get(i as usize).map(|s| s.to_string()).unwrap_or_else(|| format!("mode {i}"))
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn yes_no(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "?",
    }
}

fn opt_json(v: Option<bool>) -> String {
    v.map(|b| b.to_string()).unwrap_or("null".into())
}

// --- Status -------------------------------------------------------------------------------

fn print_status(b: &Brush, st: &State, firmware: &str, json: bool) {
    if json {
        let modes: Vec<String> = st
            .modes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                format!(
                    "{{\"mode\":{},\"strength\":{},\"range\":{},\"speed\":{}}}",
                    json_str(MODE_ARGS[i]),
                    m.strength,
                    m.range,
                    m.speed
                )
            })
            .collect();
        println!(
            "{{\"name\":{},\"address\":{},\"firmware\":{},\"battery\":{},\"mode\":{},\"modes\":[{}],\
             \"duration\":{},\"reminder\":{},\"high_frequency\":{},\"airplane\":{},\"running\":{}}}",
            json_str(&b.device.name),
            json_str(&b.device.address),
            json_str(firmware),
            st.battery,
            json_str(MODE_ARGS.get(st.mode as usize).copied().unwrap_or("?")),
            modes.join(","),
            st.duration,
            st.reminder,
            opt_json(st.high_frequency),
            st.airplane,
            opt_json(st.running),
        );
        return;
    }
    println!("{:<17} {} ({})", "Brush", b.device.name, b.device.address);
    println!("{:<17} {firmware}", "Firmware");
    println!("{:<17} {}%", "Battery", st.battery);
    println!("{:<17} {}", "Active mode", mode_name(st.mode));
    println!("{:<17} {}", "Brushing time", duration_text(st.duration));
    println!("{:<17} {}", "30 s reminder", switch_name(st.reminder));
    if let Some(hf) = st.high_frequency {
        println!("{:<17} {}", "High frequency", switch_name(hf));
    }
    println!("{:<17} {}", "Airplane mode", switch_name(st.airplane));
    if st.running.is_some() {
        println!("{:<17} {}", "Brushing", yes_no(st.running));
    }
    println!();
    println!("  {:<18} {:>8} {:>6} {:>6}", "Settings", "strength", "range", "speed");
    for (i, m) in st.modes.iter().enumerate() {
        if i == HF && st.high_frequency != Some(true) {
            continue;
        }
        let mark = if i == st.mode as usize { "▸" } else { " " };
        println!("{mark} {:<18} {:>8} {:>6} {:>6}", MODE_NAMES[i], m.strength, m.range, m.speed);
    }
}

// --- Commands that change something -------------------------------------------------------

fn mode(b: &mut Brush, action: ModeAction) -> Res<()> {
    match action {
        ModeAction::Get => {
            let st = b.state()?;
            let m = st.modes.get(st.mode as usize).ok_or("unknown active mode")?;
            println!("{}: {}", mode_name(st.mode), settings_text(m));
        }
        ModeAction::Set { mode } => {
            b.set_mode(mode)?;
            println!("{}", MODE_NAMES[mode]);
        }
        ModeAction::Tune { mode, strength, range, speed } => {
            if strength.is_none() && range.is_none() && speed.is_none() {
                return Err("nothing to change: give --strength, --range or --speed".into());
            }
            let (_, new) = b.tune(mode, strength, range, speed)?;
            println!("{}: {}", MODE_NAMES[mode], settings_text(&new));
        }
    }
    Ok(())
}

fn duration(b: &mut Brush, action: DurationAction) -> Res<()> {
    match action {
        DurationAction::Get => println!("{}", duration_text(b.state()?.duration)),
        DurationAction::Set { duration } => {
            b.set_duration(duration)?;
            println!("{}", duration_text(duration));
        }
    }
    Ok(())
}

fn reminder(b: &mut Brush, action: Switch) -> Res<()> {
    match action {
        Switch::Get => println!("{}", switch_name(b.state()?.reminder)),
        a => {
            let on = matches!(a, Switch::On);
            b.set_reminder(on)?;
            println!("{}", switch_name(on));
        }
    }
    Ok(())
}

fn high_freq(b: &mut Brush, action: Switch) -> Res<()> {
    match action {
        Switch::Get => {
            let hf = b.state()?.high_frequency.ok_or("this brush does not report high frequency")?;
            println!("{}", switch_name(hf));
        }
        a => {
            let on = matches!(a, Switch::On);
            b.set_high_frequency(on)?;
            println!("{}", switch_name(on));
        }
    }
    Ok(())
}

fn power(b: &mut Brush, on: bool) -> Res<()> {
    let st = b.set_power(on)?;
    if on {
        println!("brushing: {}, {}", mode_name(st.mode), duration_text(st.duration));
    } else {
        println!("stopped");
    }
    Ok(())
}

// --- Watching -----------------------------------------------------------------------------

fn changes(old: &State, new: &State) -> Vec<String> {
    let mut out = Vec::new();
    let mut diff = |label: &str, a: String, b: String| {
        if a != b {
            out.push(format!("{label} {a} → {b}"));
        }
    };
    diff("mode", mode_name(old.mode), mode_name(new.mode));
    diff("battery", format!("{}%", old.battery), format!("{}%", new.battery));
    diff("brushing", yes_no(old.running).into(), yes_no(new.running).into());
    diff("time", duration_text(old.duration), duration_text(new.duration));
    diff("reminder", switch_name(old.reminder).into(), switch_name(new.reminder).into());
    diff("high frequency", yes_no(old.high_frequency).into(), yes_no(new.high_frequency).into());
    diff("airplane", switch_name(old.airplane).into(), switch_name(new.airplane).into());
    for (i, (a, b)) in old.modes.iter().zip(&new.modes).enumerate() {
        diff(MODE_NAMES[i], settings_text(a), settings_text(b));
    }
    out
}

fn watch(b: &mut Brush, raw: bool) -> Res<()> {
    let start = Instant::now();
    let mut last = b.state()?;
    eprintln!("connected to {}; Ctrl+C to stop", b.device.name);
    println!(
        "{:>7.1}s  {}, {}, battery {}%",
        0.0,
        mode_name(last.mode),
        duration_text(last.duration),
        last.battery
    );
    while !signals::stop_requested() {
        let f = match b.next_frame(Duration::from_millis(250)) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(e) if e == "interrupted" => break,
            Err(e) => return Err(e),
        };
        let t = start.elapsed().as_secs_f64();
        match f {
            Err(e) => println!("{t:>7.1}s  unreadable frame: {e}"),
            Ok(f) if raw => println!("{t:>7.1}s  {}", frame::hex(&frame::raw(&f))),
            Ok(f) if f.cmd == cmd::STATE => match decode::state(&f.payload) {
                Ok(st) => {
                    let c = changes(&last, &st);
                    println!(
                        "{t:>7.1}s  state{}",
                        if c.is_empty() { ", unchanged".into() } else { format!(": {}", c.join(", ")) }
                    );
                    last = st;
                }
                Err(e) => println!("{t:>7.1}s  state: {e}"),
            },
            Ok(f) if f.cmd == cmd::VERSION => println!("{t:>7.1}s  firmware {}", decode::version(&f.payload)),
            Ok(f) => println!("{t:>7.1}s  command {:#04x}: {}", f.cmd, frame::hex(&f.payload)),
        }
    }
    Ok(())
}

// --- Scanning -----------------------------------------------------------------------------

fn scan(seconds: u64) -> Res<()> {
    let mut ble = bluez::Ble::open()?;
    eprintln!("listening for {seconds} s; a brush shows up while it is awake (press its button)");
    let found =
        ble.discover(&|name, _| brush::is_laifen(name), Duration::from_secs(seconds), false, &mut |d| {
            let support = if d.name.starts_with(brush::NAME_PREFIX) {
                "Laifen Wave, supported"
            } else {
                "not supported"
            };
            let rssi = d.rssi.map(|r| format!("{r} dBm")).unwrap_or_default();
            println!("{}  {:<20} {:>8}  {support}", d.address, d.name, rssi);
        })?;
    if found.is_empty() {
        eprintln!("no Laifen device seen");
    }
    Ok(())
}

// --- Main ---------------------------------------------------------------------------------

fn run(cli: Cli) -> Res<()> {
    let opts = Options {
        mac: cli.mac.or_else(|| std::env::var("LAIFEN_MAC").ok().filter(|m| !m.is_empty())),
        wait: Duration::from_secs(cli.wait),
        timeout: Duration::from_millis(cli.timeout),
        quiet: false,
    };
    match cli.command {
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "laifen", &mut std::io::stdout());
            Ok(())
        }
        Command::Scan { seconds } => scan(seconds),
        Command::Shell => shell(&opts),
        Command::Tui { demo, blocks, finish } => {
            let finish = match finish {
                FinishArg::Steel => wave::Finish::Steel,
                FinishArg::Aluminium => wave::Finish::Aluminium,
                FinishArg::White => wave::Finish::White,
            };
            // The view waits as long as it takes for the brush to wake, quietly.
            let opts = Options { wait: Duration::from_secs(24 * 3600), quiet: true, ..opts };
            tui::run(tui::TuiOptions { demo, blocks, finish }, opts)
        }
        command => {
            let mut b = Brush::connect(&opts)?;
            let result = execute(&mut b, command);
            b.close();
            result
        }
    }
}

/// A command that talks to a connected brush.
fn execute(b: &mut Brush, command: Command) -> Res<()> {
    match command {
        Command::Status { json } => {
            let st = b.state()?;
            let firmware = b.version()?;
            print_status(b, &st, &firmware, json);
            Ok(())
        }
        Command::Battery { json } => {
            let level = b.state()?.battery;
            if json {
                println!("{{\"level\":{level}}}");
            } else {
                println!("{level}%");
            }
            Ok(())
        }
        Command::Mode { action } => mode(b, action),
        Command::Duration { action } => duration(b, action),
        Command::Reminder(a) => reminder(b, a.action),
        Command::HighFreq(a) => high_freq(b, a.action),
        Command::Start => power(b, true),
        Command::Stop => power(b, false),
        Command::Watch { raw } => watch(b, raw),
        Command::Scan { .. } | Command::Shell | Command::Tui { .. } | Command::Completions { .. } => {
            Err("not available inside the shell".into())
        }
    }
}

// --- Shell --------------------------------------------------------------------------------

/// Waits for a line on stdin while watching for Ctrl+C (a plain read would restart after the
/// signal and keep the brush connected). None at the end of input or on Ctrl+C.
fn read_line() -> Option<String> {
    loop {
        if signals::stop_requested() {
            return None;
        }
        let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd, for the duration of the call.
        let ready = unsafe { libc::poll(&mut fd, 1, 200) };
        if ready > 0 {
            let mut line = String::new();
            return match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line),
            };
        }
    }
}

fn shell(opts: &Options) -> Res<()> {
    use std::io::{IsTerminal, Write};
    let interactive = std::io::stdin().is_terminal();
    let mut brush = Some(Brush::connect(opts)?);
    if interactive {
        eprintln!(
            "connected to {}. One command per line (status, mode set 2, duration set 2m30...), quit to leave.",
            brush.as_ref().map(|b| b.device.name.as_str()).unwrap_or("")
        );
    }
    let mut failures = 0;
    loop {
        if interactive {
            eprint!("laifen> ");
            let _ = std::io::stderr().flush();
        }
        let Some(line) = read_line() else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() || words[0].starts_with('#') {
            continue;
        }
        if !interactive {
            println!("laifen> {}", words.join(" "));
        }
        match words[0] {
            "quit" | "exit" => break,
            "sleep" => {
                match words.get(1).and_then(|w| w.parse::<f64>().ok()).filter(|s| (0.0..=3600.0).contains(s))
                {
                    Some(secs) => {
                        let until = Instant::now() + Duration::from_secs_f64(secs);
                        while Instant::now() < until && !signals::stop_requested() {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                    None => {
                        eprintln!("laifen: sleep SECONDS");
                        failures += 1;
                    }
                }
                continue;
            }
            _ => {}
        }
        let cli = match Cli::try_parse_from(std::iter::once("laifen").chain(words.iter().copied())) {
            Ok(cli) => cli,
            Err(e) => {
                let _ = e.print();
                if e.use_stderr() {
                    failures += 1;
                }
                continue;
            }
        };
        if matches!(
            cli.command,
            Command::Shell | Command::Scan { .. } | Command::Tui { .. } | Command::Completions { .. }
        ) {
            eprintln!("laifen: not available inside the shell");
            failures += 1;
            continue;
        }
        if brush.is_none() {
            match Brush::connect(opts) {
                Ok(b) => brush = Some(b),
                Err(e) => {
                    eprintln!("laifen: {e}");
                    failures += 1;
                    continue;
                }
            }
        }
        let Some(b) = brush.as_mut() else { continue };
        if let Err(e) = execute(b, cli.command) {
            eprintln!("laifen: {e}");
            failures += 1;
            if e.contains("disconnected") || e.contains("Not connected") {
                // The brush went to sleep: reconnect for the next command.
                brush = None;
            }
        }
        if signals::stop_requested() {
            break;
        }
    }
    if let Some(b) = brush {
        b.close();
    }
    match failures {
        0 => Ok(()),
        n => Err(format!("{n} command(s) failed")),
    }
}

fn main() -> ExitCode {
    signals::install();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("laifen: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn durations() {
        for (s, v) in [
            ("150", 150),
            ("150s", 150),
            ("2m30", 150),
            ("2m30s", 150),
            ("2:30", 150),
            ("3m", 180),
            ("1M", 60),
        ] {
            assert_eq!(duration_arg(s), Ok(v), "{s}");
        }
        for s in ["100", "30", "330", "2m15", "abc", "m", ""] {
            assert!(duration_arg(s).is_err(), "{s}");
        }
    }

    #[test]
    fn modes() {
        assert_eq!(mode_arg("1"), Ok(0));
        assert_eq!(mode_arg("HF"), Ok(3));
        assert!(mode_arg("4").is_err());
    }

    #[test]
    fn status_lines_name_the_changes() {
        let a =
            decode::state(&[1, 1, 10, 7, 10, 10, 10, 4, 10, 10, 20, 1, 1, 0, 93, 1, 0, 120, 1, 0]).unwrap();
        let mut b = a.clone();
        b.mode = 2;
        b.running = Some(true);
        assert_eq!(changes(&a, &b), vec!["mode Mode 2 → Mode 3", "brushing no → yes"]);
    }
}
