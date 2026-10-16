//! A minimal D-Bus client on the system bus socket: method calls, their replies, and signals.
//! Just what talking to BlueZ needs, without a D-Bus library.
//!
//! Wire format from the D-Bus specification ("Message Protocol"). Messages are written
//! little-endian and read in either byte order. Unix file descriptors are not supported.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

const MAX_MESSAGE: usize = 1 << 24;

/// A D-Bus value. Arrays keep their element signature, so that an empty one can be written.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Sig(String),
    Fd(u32),
    Array(String, Vec<Value>),
    Struct(Vec<Value>),
    Entry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn signature(&self) -> String {
        match self {
            Value::Byte(_) => "y".into(),
            Value::Bool(_) => "b".into(),
            Value::I16(_) => "n".into(),
            Value::U16(_) => "q".into(),
            Value::I32(_) => "i".into(),
            Value::U32(_) => "u".into(),
            Value::I64(_) => "x".into(),
            Value::U64(_) => "t".into(),
            Value::F64(_) => "d".into(),
            Value::Str(_) => "s".into(),
            Value::Path(_) => "o".into(),
            Value::Sig(_) => "g".into(),
            Value::Fd(_) => "h".into(),
            Value::Array(elem, _) => format!("a{elem}"),
            Value::Struct(fields) => format!("({})", fields.iter().map(Value::signature).collect::<String>()),
            Value::Entry(k, v) => format!("{{{}{}}}", k.signature(), v.signature()),
            Value::Variant(_) => "v".into(),
        }
    }

    /// `a{sv}` from (name, value) pairs; each value is wrapped in a variant.
    pub fn dict(pairs: Vec<(&str, Value)>) -> Value {
        let entries = pairs
            .into_iter()
            .map(|(k, v)| Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v)))))
            .collect();
        Value::Array("{sv}".into(), entries)
    }

    pub fn bytes(b: &[u8]) -> Value {
        Value::Array("y".into(), b.iter().map(|&x| Value::Byte(x)).collect())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self.unwrapped() {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.unwrapped() {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        Some(match self.unwrapped() {
            Value::Byte(x) => *x as i64,
            Value::I16(x) => *x as i64,
            Value::U16(x) => *x as i64,
            Value::I32(x) => *x as i64,
            Value::U32(x) => *x as i64,
            Value::I64(x) => *x,
            Value::U64(x) => *x as i64,
            _ => return None,
        })
    }

    /// The bytes of an `ay`.
    pub fn as_bytes(&self) -> Option<Vec<u8>> {
        match self.unwrapped() {
            Value::Array(sig, items) if sig == "y" => {
                items.iter().map(|v| if let Value::Byte(b) = v { Some(*b) } else { None }).collect()
            }
            _ => None,
        }
    }

    /// The entries of a dictionary (`a{..}`), as (key, value) pairs, variants unwrapped.
    pub fn entries(&self) -> Vec<(&Value, &Value)> {
        match self.unwrapped() {
            Value::Array(_, items) => items
                .iter()
                .filter_map(|e| if let Value::Entry(k, v) = e { Some((&**k, v.unwrapped())) } else { None })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The value of key `name` in an `a{sv}`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries().into_iter().find(|(k, _)| k.as_str() == Some(name)).map(|(_, v)| v)
    }

    pub fn unwrapped(&self) -> &Value {
        match self {
            Value::Variant(inner) => inner.unwrapped(),
            v => v,
        }
    }
}

// ------------------------------------------------------------------ signatures

/// End (exclusive) of the single complete type starting at `sig[i]`.
fn type_end(sig: &[u8], i: usize) -> Res<usize> {
    match sig.get(i) {
        None => Err("truncated signature".into()),
        Some(b'a') => type_end(sig, i + 1),
        Some(b'(') => {
            let mut j = i + 1;
            while sig.get(j) != Some(&b')') {
                j = type_end(sig, j)?;
            }
            Ok(j + 1)
        }
        Some(b'{') => {
            let j = type_end(sig, type_end(sig, i + 1)?)?;
            if sig.get(j) != Some(&b'}') {
                return Err("bad dict entry signature".into());
            }
            Ok(j + 1)
        }
        Some(c) if b"ybnqiuxtdsoghv".contains(c) => Ok(i + 1),
        Some(c) => Err(format!("unsupported type code {:?}", *c as char)),
    }
}

fn alignment(code: u8) -> usize {
    match code {
        b'y' | b'g' | b'v' => 1,
        b'n' | b'q' => 2,
        b'b' | b'i' | b'u' | b's' | b'o' | b'a' | b'h' => 4,
        _ => 8, // x t d ( {
    }
}

/// Splits a signature into its complete types.
fn split(sig: &str) -> Res<Vec<&str>> {
    let b = sig.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let j = type_end(b, i)?;
        out.push(&sig[i..j]);
        i = j;
    }
    Ok(out)
}

// ------------------------------------------------------------------ writing

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn pad(&mut self, align: usize) {
        while !self.buf.len().is_multiple_of(align) {
            self.buf.push(0);
        }
    }

    fn u32_at(&mut self, pos: usize, v: u32) {
        self.buf[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn string(&mut self, s: &str) {
        self.pad(4);
        self.buf.extend((s.len() as u32).to_le_bytes());
        self.buf.extend(s.as_bytes());
        self.buf.push(0);
    }

    fn put(&mut self, v: &Value) {
        match v {
            Value::Byte(x) => self.buf.push(*x),
            Value::Bool(x) => {
                self.pad(4);
                self.buf.extend((*x as u32).to_le_bytes());
            }
            Value::I16(x) => {
                self.pad(2);
                self.buf.extend(x.to_le_bytes());
            }
            Value::U16(x) => {
                self.pad(2);
                self.buf.extend(x.to_le_bytes());
            }
            Value::I32(x) => {
                self.pad(4);
                self.buf.extend(x.to_le_bytes());
            }
            Value::U32(x) | Value::Fd(x) => {
                self.pad(4);
                self.buf.extend(x.to_le_bytes());
            }
            Value::I64(x) => {
                self.pad(8);
                self.buf.extend(x.to_le_bytes());
            }
            Value::U64(x) => {
                self.pad(8);
                self.buf.extend(x.to_le_bytes());
            }
            Value::F64(x) => {
                self.pad(8);
                self.buf.extend(x.to_le_bytes());
            }
            Value::Str(s) | Value::Path(s) => self.string(s),
            Value::Sig(s) => {
                self.buf.push(s.len() as u8);
                self.buf.extend(s.as_bytes());
                self.buf.push(0);
            }
            Value::Array(elem, items) => {
                self.pad(4);
                let len_at = self.buf.len();
                self.buf.extend([0; 4]);
                self.pad(alignment(elem.as_bytes()[0]));
                let start = self.buf.len();
                for item in items {
                    self.put(item);
                }
                let len = (self.buf.len() - start) as u32;
                self.u32_at(len_at, len);
            }
            Value::Struct(fields) => {
                self.pad(8);
                for f in fields {
                    self.put(f);
                }
            }
            Value::Entry(k, v) => {
                self.pad(8);
                self.put(k);
                self.put(v);
            }
            Value::Variant(inner) => {
                self.put(&Value::Sig(inner.signature()));
                self.put(inner);
            }
        }
    }
}

pub const METHOD_CALL: u8 = 1;
pub const METHOD_RETURN: u8 = 2;
pub const ERROR: u8 = 3;
pub const SIGNAL: u8 = 4;

/// A method call, ready to send.
pub fn method_call(
    serial: u32,
    dest: &str,
    path: &str,
    iface: &str,
    member: &str,
    args: &[Value],
) -> Vec<u8> {
    let mut body = Writer { buf: Vec::new() };
    for a in args {
        body.put(a);
    }
    let sig: String = args.iter().map(Value::signature).collect();
    let field = |code: u8, v: Value| Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(v))]);
    let mut fields = vec![
        field(1, Value::Path(path.into())),
        field(3, Value::Str(member.into())),
        field(6, Value::Str(dest.into())),
    ];
    if !iface.is_empty() {
        fields.insert(1, field(2, Value::Str(iface.into())));
    }
    if !sig.is_empty() {
        fields.push(field(8, Value::Sig(sig)));
    }
    let mut w = Writer { buf: vec![b'l', METHOD_CALL, 0, 1] };
    w.buf.extend((body.buf.len() as u32).to_le_bytes());
    w.buf.extend(serial.to_le_bytes());
    w.put(&Value::Array("(yv)".into(), fields));
    w.pad(8);
    w.buf.extend(body.buf);
    w.buf
}

// ------------------------------------------------------------------ reading

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    big: bool,
}

impl Reader<'_> {
    fn align(&mut self, a: usize) -> Res<()> {
        let p = self.pos.div_ceil(a) * a;
        if p > self.buf.len() {
            return Err("truncated message".into());
        }
        self.pos = p;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Res<&[u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()).ok_or("truncated message")?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn fixed<const N: usize>(&mut self) -> Res<[u8; N]> {
        self.align(N)?;
        let mut a: [u8; N] = self.take(N)?.try_into().unwrap();
        if self.big {
            a.reverse();
        }
        Ok(a) // now little-endian
    }

    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    fn text(&mut self, len: usize) -> Res<String> {
        let s = self.take(len + 1)?;
        String::from_utf8(s[..len].to_vec()).map_err(|_| "invalid UTF-8 in a string".into())
    }

    /// One value of the single complete type `sig`.
    fn value(&mut self, sig: &str, depth: u32) -> Res<Value> {
        if depth > 64 {
            return Err("values nested too deeply".into());
        }
        let b = sig.as_bytes();
        Ok(match b[0] {
            b'y' => Value::Byte(self.take(1)?[0]),
            b'b' => Value::Bool(self.u32()? != 0),
            b'n' => Value::I16(i16::from_le_bytes(self.fixed::<2>()?)),
            b'q' => Value::U16(u16::from_le_bytes(self.fixed::<2>()?)),
            b'i' => Value::I32(i32::from_le_bytes(self.fixed::<4>()?)),
            b'u' => Value::U32(self.u32()?),
            b'h' => Value::Fd(self.u32()?),
            b'x' => Value::I64(i64::from_le_bytes(self.fixed::<8>()?)),
            b't' => Value::U64(u64::from_le_bytes(self.fixed::<8>()?)),
            b'd' => Value::F64(f64::from_le_bytes(self.fixed::<8>()?)),
            b's' | b'o' => {
                let len = self.u32()? as usize;
                let s = self.text(len)?;
                if b[0] == b's' {
                    Value::Str(s)
                } else {
                    Value::Path(s)
                }
            }
            b'g' => {
                let len = self.take(1)?[0] as usize;
                Value::Sig(self.text(len)?)
            }
            b'v' => {
                let len = self.take(1)?[0] as usize;
                let inner = self.text(len)?;
                if split(&inner)?.len() != 1 {
                    return Err("variant with a bad signature".into());
                }
                Value::Variant(Box::new(self.value(&inner, depth + 1)?))
            }
            b'a' => {
                let elem = &sig[1..];
                let len = self.u32()? as usize;
                self.align(alignment(elem.as_bytes()[0]))?;
                let end =
                    self.pos.checked_add(len).filter(|&e| e <= self.buf.len()).ok_or("truncated array")?;
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.value(elem, depth + 1)?);
                }
                if self.pos != end {
                    return Err("array length mismatch".into());
                }
                Value::Array(elem.into(), items)
            }
            b'(' => {
                self.align(8)?;
                let fields = split(&sig[1..sig.len() - 1])?;
                Value::Struct(fields.iter().map(|f| self.value(f, depth + 1)).collect::<Res<_>>()?)
            }
            b'{' => {
                self.align(8)?;
                let kv = split(&sig[1..sig.len() - 1])?;
                if kv.len() != 2 {
                    return Err("bad dict entry".into());
                }
                let k = self.value(kv[0], depth + 1)?;
                let v = self.value(kv[1], depth + 1)?;
                Value::Entry(Box::new(k), Box::new(v))
            }
            c => return Err(format!("unsupported type code {:?}", c as char)),
        })
    }
}

/// A message received from the bus.
#[derive(Debug, Default)]
pub struct Message {
    pub kind: u8,
    pub reply_serial: Option<u32>,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
}

/// Total length of the message at the start of `buf`, once its first 16 bytes are there.
fn message_len(buf: &[u8]) -> Res<Option<usize>> {
    if buf.len() < 16 {
        return Ok(None);
    }
    let big = match buf[0] {
        b'l' => false,
        b'B' => true,
        _ => return Err("bad byte order mark".into()),
    };
    let rd = |o: usize| {
        let a: [u8; 4] = buf[o..o + 4].try_into().unwrap();
        (if big { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) }) as usize
    };
    let header = (16 + rd(12)).div_ceil(8) * 8;
    let total = header + rd(4);
    if total > MAX_MESSAGE {
        return Err("message too large".into());
    }
    Ok(Some(total))
}

pub fn parse(buf: &[u8]) -> Res<Message> {
    let big = buf[0] == b'B';
    let mut r = Reader { buf, pos: 4, big };
    let body_len = r.u32()? as usize;
    let _serial = r.u32()?;
    let mut m = Message { kind: buf[1], ..Default::default() };
    let mut signature = String::new();
    if let Value::Array(_, fields) = r.value("a(yv)", 0)? {
        for f in fields {
            let Value::Struct(f) = f else { continue };
            let (Some(Value::Byte(code)), Some(v)) = (f.first(), f.get(1)) else { continue };
            let v = v.unwrapped();
            match code {
                1 => m.path = v.as_str().map(String::from),
                2 => m.interface = v.as_str().map(String::from),
                3 => m.member = v.as_str().map(String::from),
                4 => m.error_name = v.as_str().map(String::from),
                5 => m.reply_serial = v.as_i64().map(|x| x as u32),
                7 => m.sender = v.as_str().map(String::from),
                8 => signature = v.as_str().unwrap_or("").into(),
                _ => {}
            }
        }
    }
    r.align(8)?;
    let body = r.take(body_len)?;
    let mut br = Reader { buf: body, pos: 0, big };
    for t in split(&signature)? {
        m.body.push(br.value(t, 0)?);
    }
    Ok(m)
}

// ------------------------------------------------------------------ the connection

pub struct Bus {
    stream: UnixStream,
    serial: u32,
    rbuf: Vec<u8>,
    signals: VecDeque<Message>,
}

impl Bus {
    /// Connects to the system bus ($DBUS_SYSTEM_BUS_ADDRESS, else the usual socket).
    pub fn system() -> Res<Bus> {
        let path = std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
            .ok()
            .and_then(|a| {
                a.split(';').find_map(|t| {
                    t.strip_prefix("unix:path=").map(|p| p.split(',').next().unwrap().to_string())
                })
            })
            .unwrap_or_else(|| "/run/dbus/system_bus_socket".into());
        let mut stream = UnixStream::connect(&path).map_err(|e| format!("system bus ({path}): {e}"))?;
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let hex: String = uid.to_string().bytes().map(|b| format!("{b:02x}")).collect();
        stream.write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes()).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            stream.read_exact(&mut byte).map_err(|e| format!("system bus authentication: {e}"))?;
            line.push(byte[0]);
            if line.len() > 512 {
                return Err("system bus authentication: reply too long".into());
            }
        }
        if !line.starts_with(b"OK ") {
            return Err(format!("system bus refused us: {}", String::from_utf8_lossy(&line).trim()));
        }
        stream.write_all(b"BEGIN\r\n").map_err(|e| e.to_string())?;
        let mut bus = Bus { stream, serial: 0, rbuf: Vec::new(), signals: VecDeque::new() };
        bus.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            &[],
            5000,
        )?;
        Ok(bus)
    }

    /// Calls a method and waits up to `timeout_ms` for its reply. Signals that arrive meanwhile
    /// are kept for `next_signal`.
    pub fn call(
        &mut self,
        dest: &str,
        path: &str,
        iface: &str,
        member: &str,
        args: &[Value],
        timeout_ms: u64,
    ) -> Res<Vec<Value>> {
        self.serial = self.serial.wrapping_add(1).max(1);
        let serial = self.serial;
        let msg = method_call(serial, dest, path, iface, member, args);
        self.stream.write_all(&msg).map_err(|e| format!("system bus: {e}"))?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let Some(m) = self.receive(deadline)? else {
                return Err(format!(
                    "{member}: no answer from {dest} within {:.1} s",
                    timeout_ms as f64 / 1000.0
                ));
            };
            match m.kind {
                SIGNAL => self.signals.push_back(m),
                METHOD_RETURN if m.reply_serial == Some(serial) => return Ok(m.body),
                ERROR if m.reply_serial == Some(serial) => {
                    let name = m.error_name.unwrap_or_default();
                    let text = m.body.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                    return Err(if text.is_empty() { name } else { format!("{name}: {text}") });
                }
                _ => {}
            }
        }
    }

    /// Subscribes to signals matching a D-Bus match rule.
    pub fn add_match(&mut self, rule: &str) -> Res<()> {
        self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "AddMatch",
            &[Value::Str(rule.into())],
            5000,
        )
        .map(|_| ())
    }

    /// The next signal, or None once `timeout` has passed without one.
    pub fn next_signal(&mut self, timeout: Duration) -> Res<Option<Message>> {
        if let Some(m) = self.signals.pop_front() {
            return Ok(Some(m));
        }
        let deadline = Instant::now() + timeout;
        while let Some(m) = self.receive(deadline)? {
            if m.kind == SIGNAL {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// The next message from the bus, or None at the deadline.
    fn receive(&mut self, deadline: Instant) -> Res<Option<Message>> {
        loop {
            if let Some(n) = message_len(&self.rbuf)? {
                if self.rbuf.len() >= n {
                    let raw: Vec<u8> = self.rbuf.drain(..n).collect();
                    return parse(&raw).map(Some);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            // Short waits, so that a Ctrl+C is noticed quickly.
            let wait = (deadline - now).min(Duration::from_millis(200));
            self.stream.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("the system bus closed the connection".into()),
                Ok(n) => self.rbuf.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) =>
                {
                    if crate::signals::stop_requested() {
                        return Err("interrupted".into());
                    }
                }
                Err(e) => return Err(format!("system bus: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_split_into_complete_types() {
        assert_eq!(split("a{sv}oas(yv)y").unwrap(), vec!["a{sv}", "o", "as", "(yv)", "y"]);
        assert_eq!(split("a{oa{sa{sv}}}").unwrap(), vec!["a{oa{sa{sv}}}"]);
        assert!(split("a{s").is_err());
    }

    #[test]
    fn hello_matches_the_reference_bytes() {
        // The Hello call (serial 1), laid out by hand from the specification.
        let m = method_call(
            1,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            &[],
        );
        let mut expected = vec![b'l', 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0x6d, 0, 0, 0];
        expected.extend([1, 1, b'o', 0, 21, 0, 0, 0]);
        expected.extend(b"/org/freedesktop/DBus\0");
        expected.extend([0, 0]); // pad to 8
        expected.extend([2, 1, b's', 0, 20, 0, 0, 0]);
        expected.extend(b"org.freedesktop.DBus\0");
        expected.extend([0, 0, 0]);
        expected.extend([3, 1, b's', 0, 5, 0, 0, 0]);
        expected.extend(b"Hello\0");
        expected.extend([0, 0]);
        expected.extend([6, 1, b's', 0, 20, 0, 0, 0]);
        expected.extend(b"org.freedesktop.DBus\0");
        expected.extend([0, 0, 0]);
        assert_eq!(m, expected);
        assert_eq!(message_len(&m).unwrap(), Some(m.len()));
    }

    #[test]
    fn values_survive_a_round_trip() {
        let args = vec![
            Value::bytes(&[0xaa, 0x0a, 0, 0, 0xa0]),
            Value::dict(vec![("type", Value::Str("request".into())), ("flag", Value::Bool(true))]),
            Value::Array("{qv}".into(), vec![]),
            Value::Struct(vec![Value::I16(-60), Value::U64(7), Value::Path("/org/bluez/hci0".into())]),
        ];
        let raw = method_call(9, "org.bluez", "/", "x.Y", "Z", &args);
        let m = parse(&raw).unwrap();
        assert_eq!(m.member.as_deref(), Some("Z"));
        assert_eq!(m.body, args);
        assert_eq!(m.body[0].as_bytes().unwrap(), vec![0xaa, 0x0a, 0, 0, 0xa0]);
        assert_eq!(m.body[1].get("type").and_then(Value::as_str), Some("request"));
        assert_eq!(m.body[1].get("flag").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn big_endian_messages_are_read() {
        // A signal with body (s "x", u 258), written big-endian by hand.
        let mut b = vec![b'B', SIGNAL, 0, 1, 0, 0, 0, 12, 0, 0, 0, 5, 0, 0, 0, 18];
        b.extend([8, 1, b'g', 0, 2, b's', b'u', 0]);
        b.extend([3, 1, b's', 0, 0, 0, 0, 1, b'M', 0]);
        b.extend([0, 0, 0, 0, 0, 0]); // the header fields end at 34: pad to 40
        b.extend([0, 0, 0, 1, b'x', 0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(message_len(&b).unwrap(), Some(b.len()));
        let m = parse(&b).unwrap();
        assert_eq!(m.member.as_deref(), Some("M"));
        assert_eq!(m.body, vec![Value::Str("x".into()), Value::U32(258)]);
    }
}
