//! One session with the brush: find it, connect, ask, write and check, disconnect.

use crate::bluez::{Ble, Device, Event};
use crate::cmd;
use crate::decode::{self, Settings, State, HF, MODE_NAMES};
use crate::frame::{self, Frame, Kind, FROM_BRUSH};
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

/// The Laifen Wave advertises as "LFTB01-P-" followed by the end of its address.
pub const NAME_PREFIX: &str = "LFTB01-P";
/// Names of a Laifen brush in firmware-update mode.
const UPDATE_NAMES: [&str; 2] = ["PPlusOTA", "LFTB-S-01-OTA"];

/// What every Laifen device advertises as (brushes, the kids' brush and its base).
pub fn is_laifen(name: &str) -> bool {
    name.starts_with("LF") || name.starts_with("TC51BB") || UPDATE_NAMES.iter().any(|n| name.starts_with(n))
}

pub struct Options {
    pub mac: Option<String>,
    pub wait: Duration,
    pub timeout: Duration,
    /// No hint on stderr while waiting for the brush (the 3D view owns the screen).
    pub quiet: bool,
}

pub struct Brush {
    ble: Ble,
    pub device: Device,
    chr: String,
    timeout: Duration,
    closed: bool,
}

impl Brush {
    pub fn connect(opts: &Options) -> Res<Brush> {
        let mut ble = Ble::open()?;
        let mac = opts.mac.clone();
        let accept = move |name: &str, address: &str| match &mac {
            Some(m) => address.eq_ignore_ascii_case(m),
            None => name.starts_with(NAME_PREFIX) || UPDATE_NAMES.iter().any(|n| name.starts_with(n)),
        };
        let mut device = match ble.connected(&accept)? {
            Some(d) => d,
            None => find(&mut ble, &accept, opts)?,
        };
        if UPDATE_NAMES.iter().any(|n| device.name.starts_with(n)) {
            return Err(format!(
                "{} ({}) is in firmware-update mode; this tool does not talk to it",
                device.name, device.address
            ));
        }
        if !device.name.is_empty() && !device.name.starts_with(NAME_PREFIX) {
            return Err(format!(
                "{} ({}) is not a Laifen Wave (LFTB01-P); other Laifen brushes speak another protocol, \
                 not supported",
                device.name, device.address
            ));
        }
        // Seen once on a real brush: found by discovery right after a disconnection, then the
        // connection never came up. Cancel, wait for a fresh advertisement, try again.
        let mut attempt = 1;
        let chr = loop {
            match ble.connect(&device) {
                Ok(chr) => break chr,
                Err(e) if attempt < 3 && !e.contains("interrupted") => {
                    ble.disconnect(&device.path, None);
                    attempt += 1;
                    device = find(&mut ble, &accept, opts)?;
                }
                Err(e) => {
                    ble.disconnect(&device.path, None);
                    return Err(e);
                }
            }
        };
        Ok(Brush { ble, device, chr, timeout: opts.timeout, closed: false })
    }

    /// The next frame the brush sends, within `timeout`; Err when it disconnects.
    pub fn next_frame(&mut self, timeout: Duration) -> Res<Option<Result<Frame, String>>> {
        match self.ble.next_event(&self.device.path, &self.chr, timeout)? {
            None => Ok(None),
            Some(Event::Disconnected) => Err("the brush disconnected".into()),
            Some(Event::Notification(raw)) => Ok(Some(frame::decode(&raw))),
        }
    }

    /// Waits for the first frame that `want` accepts; others (reports) are skipped.
    fn wait_for(&mut self, what: &str, want: &dyn Fn(&Frame) -> bool) -> Res<Frame> {
        let deadline = Instant::now() + self.timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Some(Ok(f)) = self.next_frame(left)? {
                if f.kind == FROM_BRUSH && want(&f) {
                    return Ok(f);
                }
            }
        }
        Err(format!("no {what} from the brush within {:.1} s", self.timeout.as_secs_f64()))
    }

    /// Drops what the brush already sent, so that the next answer is a fresh one.
    fn drain(&mut self) -> Res<()> {
        while self.next_frame(Duration::from_millis(40))?.is_some() {}
        Ok(())
    }

    fn send(&mut self, cmd: u8, kind: Kind, payload: &[u8]) -> Res<()> {
        cmd::check(cmd, kind)?;
        self.drain()?;
        self.ble.write(&self.chr, &frame::encode(cmd, kind, payload))
    }

    pub fn state(&mut self) -> Res<State> {
        self.send(cmd::STATE, Kind::Read, &[])?;
        let f = self.wait_for("state", &|f| f.cmd == cmd::STATE)?;
        decode::state(&f.payload)
    }

    pub fn version(&mut self) -> Res<String> {
        self.send(cmd::VERSION, Kind::Read, &[])?;
        let f = self.wait_for("version", &|f| f.cmd == cmd::VERSION)?;
        Ok(decode::version(&f.payload))
    }

    /// Sends a write and requires the brush's acknowledgement, status 0.
    pub fn write(&mut self, cmd: u8, payload: &[u8]) -> Res<()> {
        self.send(cmd, Kind::Write, payload)?;
        let ack = self.wait_for("acknowledgement", &|f| f.cmd == cmd && f.payload.len() == 1)?;
        match ack.payload[0] {
            0 => Ok(()),
            s if cmd == cmd::POWER => {
                Err(format!("the brush refused to start: {}", decode::power_refusal(s)))
            }
            s => Err(format!("the brush refused the command (status {s})")),
        }
    }

    /// Reads the state back until `applied` holds. After a mode or high-frequency change the
    /// brush pushes its state, sometimes late, so a stale one may come first: one more try.
    pub fn check(&mut self, applied: &dyn Fn(&State) -> bool) -> Res<State> {
        std::thread::sleep(Duration::from_millis(500));
        let st = self.state()?;
        if applied(&st) {
            return Ok(st);
        }
        std::thread::sleep(Duration::from_millis(800));
        let st = self.state()?;
        if applied(&st) {
            Ok(st)
        } else {
            Err("the brush acknowledged the change but its state does not show it".into())
        }
    }

    // --- Changes, each checked the way the vendor app checks it, then read back --------

    /// The current state, when the vendor app would accept a change.
    fn writable(&mut self) -> Res<State> {
        let st = self.state()?;
        if st.airplane {
            return Err("the brush is in airplane mode: the vendor app refuses every change then, and so \
                        does this tool"
                .into());
        }
        Ok(st)
    }

    pub fn set_mode(&mut self, mode: usize) -> Res<State> {
        let st = self.writable()?;
        available(&st, mode)?;
        self.write(cmd::MODE, &mode_payload(mode, &st.modes[mode]))?;
        self.check(&|s| s.mode as usize == mode)
    }

    /// Changes a mode's settings (None keeps a value) without switching to it. Returns the
    /// new state and the mode's new settings.
    pub fn tune(
        &mut self,
        mode: usize,
        strength: Option<u8>,
        range: Option<u8>,
        speed: Option<u8>,
    ) -> Res<(State, Settings)> {
        let st = self.writable()?;
        available(&st, mode)?;
        let (low, high) = strength_range(mode);
        let check = |name: &str, v: Option<u8>, lo: u8, hi: u8| match v {
            Some(v) if !(lo..=hi).contains(&v) => {
                Err(format!("{name} {v}: {lo} to {hi} for {}", MODE_NAMES[mode]))
            }
            _ => Ok(()),
        };
        check("strength", strength, low, high)?;
        check("range", range, 1, 10)?;
        check("speed", speed, 1, 10)?;
        let old = st.modes[mode];
        let new = Settings {
            strength: strength.unwrap_or(old.strength),
            range: range.unwrap_or(old.range),
            speed: speed.unwrap_or(old.speed),
        };
        let active = st.mode as usize;
        self.write(cmd::MODE, &mode_payload(mode, &new))?;
        if mode != active {
            // Setting a mode also switches to it (seen on the brush): switch back.
            self.write(cmd::MODE, &mode_payload(active, &st.modes[active]))?;
        }
        let st = self.check(&|s| s.modes.get(mode) == Some(&new) && s.mode as usize == active)?;
        Ok((st, new))
    }

    /// Brushing time: 1 to 5 minutes in 30-second steps, as the vendor app offers.
    pub fn set_duration(&mut self, secs: u16) -> Res<State> {
        if !(60..=300).contains(&secs) || !secs.is_multiple_of(30) {
            return Err(format!("{secs} s: 1 to 5 minutes in 30-second steps"));
        }
        self.writable()?;
        self.write(cmd::DURATION, &secs.to_be_bytes())?;
        self.check(&|s| s.duration == secs)
    }

    pub fn set_reminder(&mut self, on: bool) -> Res<State> {
        self.writable()?;
        self.write(cmd::REMINDER, &[on as u8])?;
        self.check(&|s| s.reminder == on)
    }

    pub fn set_high_frequency(&mut self, on: bool) -> Res<State> {
        let st = self.writable()?;
        st.high_frequency.ok_or("this brush does not report high frequency")?;
        if !on && st.mode as usize == HF {
            return Err("high frequency is the active mode: switch to mode 3 first".into());
        }
        self.write(cmd::HIGH_FREQUENCY, &[on as u8])?;
        self.check(&|s| s.high_frequency == Some(on))
    }

    /// Starts or stops brushing, with the active mode.
    pub fn set_power(&mut self, on: bool) -> Res<State> {
        if on {
            self.writable()?;
        }
        self.write(cmd::POWER, &[on as u8])?;
        self.check(&|s| s.running == Some(on))
    }

    pub fn close(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if !self.closed {
            self.closed = true;
            self.ble.disconnect(&self.device.path, Some(&self.chr));
        }
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn mode_payload(i: usize, s: &Settings) -> [u8; 4] {
    [i as u8, s.strength, s.range, s.speed]
}

/// The vibration strength's range: 1-10, 11-20 in high frequency.
pub fn strength_range(mode: usize) -> (u8, u8) {
    if mode == HF {
        (11, 20)
    } else {
        (1, 10)
    }
}

fn available(st: &State, i: usize) -> Res<()> {
    if i == HF && st.high_frequency != Some(true) {
        return Err("high frequency is off: turn it on first".into());
    }
    if i >= st.modes.len() {
        return Err(format!("this brush has no {}", MODE_NAMES[i]));
    }
    Ok(())
}

/// Discovery until the brush shows up; a hint on stderr if it does not right away.
fn find(ble: &mut Ble, accept: &dyn Fn(&str, &str) -> bool, opts: &Options) -> Res<Device> {
    let quick = opts.wait.min(Duration::from_millis(1500));
    let mut found = ble.discover(accept, quick, true, &mut |_| {})?;
    if found.is_empty() && opts.wait > quick {
        if !opts.quiet {
            eprintln!(
                "looking for the brush (up to {} s); if it sleeps, press its button",
                opts.wait.as_secs()
            );
        }
        found = ble.discover(accept, opts.wait - quick, true, &mut |_| {})?;
    }
    found.into_iter().next().ok_or_else(|| {
        format!(
            "no Laifen Wave found within {} s. Press the brush's button to wake it, and close the \
             Laifen app on your phone if it is open",
            opts.wait.as_secs()
        )
    })
}
