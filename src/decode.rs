//! Payload decoders: the brush's state (command 0x0A), its firmware version (0x01), and
//! write acknowledgements.

/// Index of the high-frequency mode, after modes 1, 2 and 3 (0, 1, 2).
pub const HF: usize = 3;
pub const MODE_NAMES: [&str; 4] = ["Mode 1", "Mode 2", "Mode 3", "High frequency"];
/// Mode names on the command line.
pub const MODE_ARGS: [&str; 4] = ["1", "2", "3", "hf"];

/// The three settings of a mode, named after the vendor app's sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// "Vibration Strength": 1-10, 11-20 in high frequency.
    pub strength: u8,
    /// "Oscillation Range": 1-10.
    pub range: u8,
    /// "Oscillation Speed": 1-10.
    pub speed: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// Active mode: 0-2 for modes 1-3, 3 for high frequency.
    pub mode: u8,
    /// Settings of modes 1-3, then high frequency (absent from the 15-byte layout).
    pub modes: Vec<Settings>,
    pub airplane: bool,
    pub battery: u8,
    /// The vibration every 30 seconds.
    pub reminder: bool,
    /// Brushing time in seconds, the same for every mode.
    pub duration: u16,
    pub high_frequency: Option<bool>,
    /// The motor is running.
    pub running: Option<bool>,
    pub emergency_shutdown: Option<bool>,
    pub charging: Option<u8>,
    pub deep_clean: Option<bool>,
}

fn settings(p: &[u8], at: usize) -> Settings {
    Settings { strength: p[at], range: p[at + 1], speed: p[at + 2] }
}

/// The state, from the payload of a 0x0A frame. Known layouts: 15 bytes (three modes), and
/// 20, 21 or 23 bytes (four modes and more fields). A brush on firmware V2.0.8.Y44.303 sends 20.
pub fn state(p: &[u8]) -> Result<State, String> {
    let on = |b: u8| b == 1;
    let duration = |at: usize| u16::from_be_bytes([p[at], p[at + 1]]);
    match p.len() {
        15 => Ok(State {
            mode: p[0],
            modes: (0..3).map(|i| settings(p, 1 + 3 * i)).collect(),
            airplane: on(p[10]),
            battery: p[11],
            reminder: on(p[12]),
            duration: duration(13),
            high_frequency: None,
            running: None,
            emergency_shutdown: None,
            charging: None,
            deep_clean: None,
        }),
        20 | 21 | 23 => Ok(State {
            mode: p[0],
            modes: (0..4).map(|i| settings(p, 1 + 3 * i)).collect(),
            airplane: on(p[13]),
            battery: p[14],
            reminder: on(p[15]),
            duration: duration(16),
            high_frequency: Some(on(p[18])),
            running: Some(on(p[19])),
            emergency_shutdown: p.get(20).map(|&b| on(b)),
            charging: p.get(21).copied(),
            deep_clean: p.get(22).map(|&b| on(b)),
        }),
        n => Err(format!("unknown state layout ({n} bytes)")),
    }
}

/// The firmware version as the vendor app prints it, from the payload of a 0x01 frame: nine
/// bytes, signed decimal numbers except the fourth, a letter (`V2.0.8.Y44.303`). Other
/// lengths are shown in hex: the app's 19-byte format was not seen on a device.
pub fn version(p: &[u8]) -> String {
    if p.len() != 9 {
        return format!("unknown ({})", crate::frame::hex(p));
    }
    let n = |i: usize| (p[i] as i8).to_string();
    format!("V{}.{}.{}.{}{}{}.{}{}{}", n(0), n(1), n(2), p[3] as char, n(4), n(5), n(6), n(7), n(8))
}

/// Why the brush refused to start, from the status byte of its 0x0F answer (the vendor app's
/// messages). 0 means accepted.
pub fn power_refusal(status: u8) -> String {
    match status {
        1 => "it is charging".into(),
        2 => "it reports a fault".into(),
        3 => "its battery is low".into(),
        s => format!("status {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{decode, unhex};

    #[test]
    fn the_20_byte_state_of_a_real_brush() {
        let f = decode(&unhex("aa 0a 02 14 01 01 0a 07 0a 0a 0a 04 0a 0a 14 01 01 00 5d 01 00 78 01 00 84"))
            .unwrap();
        let s = state(&f.payload).unwrap();
        assert_eq!(s.mode, 1);
        assert_eq!(s.modes[0], Settings { strength: 1, range: 10, speed: 7 });
        assert_eq!(s.modes[1], Settings { strength: 10, range: 10, speed: 10 });
        assert_eq!(s.modes[2], Settings { strength: 4, range: 10, speed: 10 });
        assert_eq!(s.modes[HF], Settings { strength: 20, range: 1, speed: 1 });
        assert!(!s.airplane);
        assert_eq!(s.battery, 93);
        assert!(s.reminder);
        assert_eq!(s.duration, 120);
        assert_eq!(s.high_frequency, Some(true));
        assert_eq!(s.running, Some(false));
        assert_eq!(s.emergency_shutdown, None);
    }

    #[test]
    fn running_and_duration_from_real_frames() {
        // Pushed right after the motor was started, then after a duration of 150 s was set.
        let f = decode(&unhex("aa 0a 02 14 01 01 0a 07 0a 0a 0a 04 0a 0a 14 01 01 00 5c 01 00 78 01 01 84"))
            .unwrap();
        assert_eq!(state(&f.payload).unwrap().running, Some(true));
        let f = decode(&unhex("aa 0a 02 14 01 01 0a 07 0a 0a 0a 04 0a 0a 14 01 01 00 5d 01 00 96 01 00 6a"))
            .unwrap();
        assert_eq!(state(&f.payload).unwrap().duration, 150);
    }

    #[test]
    fn other_layouts() {
        let mut p15 = vec![2, 1, 3, 1, 1, 6, 1, 1, 10, 1, 0, 80, 1, 0x01, 0x2c];
        let s = state(&p15).unwrap();
        assert_eq!((s.mode, s.modes.len(), s.battery, s.duration), (2, 3, 80, 300));
        assert_eq!(s.high_frequency, None);
        p15.push(0);
        assert!(state(&p15).is_err());
        let p23: Vec<u8> = [vec![0; 13], vec![1, 50, 0, 0, 60, 0, 1, 0, 2, 1]].concat();
        let s = state(&p23).unwrap();
        assert!(s.airplane && s.emergency_shutdown == Some(false) && s.deep_clean == Some(true));
        assert_eq!(s.charging, Some(2));
    }

    #[test]
    fn version_as_the_app_prints_it() {
        assert_eq!(version(&unhex("02 00 08 59 04 04 03 00 03")), "V2.0.8.Y44.303");
        assert!(version(&[1, 2]).starts_with("unknown"));
    }
}
