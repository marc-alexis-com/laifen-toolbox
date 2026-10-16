//! Frames of the Laifen Wave (LFTB01-P), the vendor app's "old" protocol:
//!
//! ```text
//! AA | command | kind | length | payload | XOR of every previous byte
//! ```
//!
//! `kind` is 00 for a read and 01 for a write from the computer, 02 in everything the brush
//! sends. One frame per Bluetooth notification.

pub const HEADER: u8 = 0xAA;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read = 0,
    Write = 1,
}

/// `kind` of every frame sent by the brush (answers, acknowledgements, reports).
pub const FROM_BRUSH: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

pub fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |x, b| x ^ b)
}

pub fn encode(cmd: u8, kind: Kind, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![HEADER, cmd, kind as u8, payload.len() as u8];
    f.extend_from_slice(payload);
    f.push(xor(&f));
    f
}

pub fn decode(raw: &[u8]) -> Result<Frame, String> {
    if raw.len() < 5 {
        return Err(format!("frame too short ({} bytes)", raw.len()));
    }
    if raw[0] != HEADER {
        return Err(format!("frame does not start with AA: {}", hex(raw)));
    }
    let len = raw[3] as usize;
    if raw.len() != len + 5 {
        return Err(format!("length byte says {len}, the frame carries {}: {}", raw.len() - 5, hex(raw)));
    }
    if xor(&raw[..raw.len() - 1]) != raw[raw.len() - 1] {
        return Err(format!("bad checksum: {}", hex(raw)));
    }
    Ok(Frame { cmd: raw[1], kind: raw[2], payload: raw[4..4 + len].to_vec() })
}

/// The bytes of a received frame, as they came (its checksum was verified on decoding).
pub fn raw(f: &Frame) -> Vec<u8> {
    let mut b = vec![HEADER, f.cmd, f.kind, f.payload.len() as u8];
    b.extend_from_slice(&f.payload);
    b.push(xor(&b));
    b
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
pub fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_match_the_frames_sent_to_a_real_brush() {
        // Every frame below was sent to an LFTB01-P (firmware V2.0.8.Y44.303) and acknowledged.
        assert_eq!(hex(&encode(0x0A, Kind::Read, &[])), "aa 0a 00 00 a0");
        assert_eq!(hex(&encode(0x01, Kind::Read, &[])), "aa 01 00 00 ab");
        assert_eq!(hex(&encode(0x05, Kind::Write, &[0x00, 0x96])), "aa 05 01 02 00 96 3a");
        assert_eq!(hex(&encode(0x04, Kind::Write, &[1, 10, 10, 9])), "aa 04 01 04 01 0a 0a 09 a3");
        assert_eq!(hex(&encode(0x0B, Kind::Write, &[0])), "aa 0b 01 01 00 a1");
        assert_eq!(hex(&encode(0x0E, Kind::Write, &[1])), "aa 0e 01 01 01 a5");
        assert_eq!(hex(&encode(0x0F, Kind::Write, &[1])), "aa 0f 01 01 01 a4");
        assert_eq!(hex(&encode(0x0F, Kind::Write, &[0])), "aa 0f 01 01 00 a5");
    }

    #[test]
    fn frames_from_the_brush_decode() {
        let f = decode(&unhex("aa 0b 02 01 00 a2")).unwrap();
        assert_eq!(f, Frame { cmd: 0x0B, kind: FROM_BRUSH, payload: vec![0] });
        let f = decode(&unhex("aa 01 02 09 02 00 08 59 04 04 03 00 03 f3")).unwrap();
        assert_eq!(f.payload.len(), 9);
    }

    #[test]
    fn damaged_frames_are_refused() {
        assert!(decode(&unhex("aa 0b 02 01 00 a3")).unwrap_err().contains("checksum"));
        assert!(decode(&unhex("aa 0b 02 02 00 a2")).unwrap_err().contains("length"));
        assert!(decode(&unhex("ab 0b 02 01 00 a2")).is_err());
        assert!(decode(&unhex("aa 0b 02")).is_err());
    }
}
