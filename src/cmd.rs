//! Command codes of the LFTB01-P, and the safety policy: what may leave the computer.
//!
//! Only reads and writes that were sent to a real brush, read back and restored are allowed.
//! Everything else is refused before a frame is even built.

use crate::frame::Kind;

pub const VERSION: u8 = 0x01;
/// Select a mode and set its vibration strength, oscillation range and speed.
pub const MODE: u8 = 0x04;
pub const DURATION: u8 = 0x05;
pub const STATE: u8 = 0x0A;
/// The vibration every 30 seconds.
pub const REMINDER: u8 = 0x0B;
pub const HIGH_FREQUENCY: u8 = 0x0E;
/// Motor on / off.
pub const POWER: u8 = 0x0F;

/// Why a command code is refused.
fn refusal(cmd: u8) -> &'static str {
    match cmd {
        0x0D => "factory reset: never sent by this tool",
        0x06 => "brush-head calibration: never sent by this tool",
        0x02 => "set the date: never used by the vendor app, the year encoding is unknown",
        0x07 => "airplane mode: it may turn the brush's Bluetooth off, untested",
        0x14 => "emergency shutdown: untested, and refused by the vendor app on firmware 2.0.7 and 2.0.8",
        0x16 => "deep / gentle clean: untested, not offered by the vendor app on firmware 2.0.x",
        0x0C => "restore a mode's defaults: untested",
        0x09 => "brushing history: reading it may clear it on the brush, untested",
        _ => "not tested on a real brush",
    }
}

pub fn check(cmd: u8, kind: Kind) -> Result<(), String> {
    match (kind, cmd) {
        (Kind::Read, VERSION | STATE) => Ok(()),
        (Kind::Write, MODE | DURATION | REMINDER | HIGH_FREQUENCY | POWER) => Ok(()),
        _ => Err(format!("refusing command {cmd:#04x} ({:?}): {}", kind, refusal(cmd))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_commands_are_refused() {
        for cmd in [0x0D, 0x06, 0x02, 0x07, 0x14, 0x16, 0x0C, 0x09, 0x10, 0x03, 0x08] {
            assert!(check(cmd, Kind::Write).is_err(), "write {cmd:#04x}");
            assert!(check(cmd, Kind::Read).is_err(), "read {cmd:#04x}");
        }
        assert!(check(0x0D, Kind::Write).unwrap_err().contains("factory reset"));
    }

    #[test]
    fn tested_commands_pass() {
        for cmd in [MODE, DURATION, REMINDER, HIGH_FREQUENCY, POWER] {
            assert!(check(cmd, Kind::Write).is_ok());
        }
        assert!(check(STATE, Kind::Read).is_ok());
        assert!(check(VERSION, Kind::Read).is_ok());
        // Reads and writes are not interchangeable.
        assert!(check(STATE, Kind::Write).is_err());
        assert!(check(POWER, Kind::Read).is_err());
    }
}
