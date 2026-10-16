// Taken from nothing-toolbox (same author, MIT): https://github.com/marc-alexis-com/nothing-toolbox
//! A small zlib (RFC 1950/1951) compressor for the frames sent to kitty (`o=z`): greedy LZ77
//! with hash chains and the fixed Huffman codes. It is tuned for pictures scaled up by whole
//! pixels, where most of the work is copying the row above: the caller passes the row length
//! as a hint and that distance is tried first.

const WINDOW: usize = 32768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
/// Positions tried along a hash chain before settling for the best match so far.
const MAX_CHAIN: usize = 8;
/// Positions inside longer matches are not indexed: it costs time and rarely pays.
const INDEX_LIMIT: usize = 16;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195,
    227, 258,
];
const LEN_EXTRA: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073,
    4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Bits are packed from the least significant end (RFC 1951, 3.1.1).
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, len: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += len;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Huffman codes go most significant bit first.
    fn put_code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Fixed literal/length code (RFC 1951, 3.2.6).
fn symbol(w: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => w.put_code(0x30 + sym, 8),
        144..=255 => w.put_code(0x190 + sym - 144, 9),
        256..=279 => w.put_code(sym - 256, 7),
        _ => w.put_code(0xc0 + sym - 280, 8),
    }
}

/// Index of the last base not above `v`.
fn bucket(bases: &[u16], v: usize) -> usize {
    bases.iter().rposition(|&b| b as usize <= v).unwrap_or(0)
}

fn copy(w: &mut BitWriter, len: usize, dist: usize) {
    let l = bucket(&LEN_BASE, len);
    symbol(w, 257 + l as u32);
    w.put((len - LEN_BASE[l] as usize) as u32, LEN_EXTRA[l] as u32);
    let d = bucket(&DIST_BASE, dist);
    w.put_code(d as u32, 5);
    w.put((dist - DIST_BASE[d] as usize) as u32, DIST_EXTRA[d] as u32);
}

fn hash(data: &[u8], i: usize) -> usize {
    let v = (data[i] as u32) | (data[i + 1] as u32) << 8 | (data[i + 2] as u32) << 16;
    (v.wrapping_mul(0x9e37_79b1) >> (32 - HASH_BITS)) as usize
}

fn match_len(data: &[u8], a: usize, b: usize) -> usize {
    let max = MAX_MATCH.min(data.len() - b);
    (0..max).find(|&k| data[a + k] != data[b + k]).unwrap_or(max)
}

/// Raw deflate stream (a single fixed-Huffman block). `hint` is a distance worth trying
/// first (the length of a picture row), 0 for none.
pub fn deflate(data: &[u8], hint: usize) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(data.len() / 4 + 16), acc: 0, n: 0 };
    w.put(1, 1); // last block
    w.put(1, 2); // fixed Huffman codes
                 // head: last position + 1 for each hash; prev: the previous one with the same hash.
    let mut head = vec![0u32; 1 << HASH_BITS];
    let mut prev = vec![0u32; WINDOW];
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        if i + MIN_MATCH <= data.len() {
            let h = hash(data, i);
            prev[i % WINDOW] = head[h];
            head[h] = i as u32 + 1;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best, mut dist) = (0, 0);
        if i + MIN_MATCH <= data.len() {
            if hint > 0 && hint <= i && hint <= WINDOW {
                best = match_len(data, i - hint, i);
                dist = hint;
            }
            let mut cand = head[hash(data, i)] as usize;
            let mut tries = 0;
            while cand > 0 && best < MAX_MATCH && tries < MAX_CHAIN {
                let j = cand - 1;
                if i - j > WINDOW {
                    break;
                }
                let l = match_len(data, j, i);
                if l > best {
                    (best, dist) = (l, i - j);
                }
                cand = prev[j % WINDOW] as usize;
                tries += 1;
            }
        }
        if best >= MIN_MATCH {
            copy(&mut w, best, dist);
            for k in i..i + best.min(INDEX_LIMIT) {
                insert(k, &mut head, &mut prev);
            }
            i += best;
        } else {
            symbol(&mut w, data[i] as u32);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    symbol(&mut w, 256);
    w.finish()
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// A zlib stream: header, deflate data, Adler-32 of the input.
pub fn compress(data: &[u8], hint: usize) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.extend(deflate(data, hint));
    out.extend(adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A decoder for what `deflate` produces (fixed Huffman blocks only), to check round trips.
    pub fn inflate_fixed(src: &[u8]) -> Vec<u8> {
        let mut pos = 0usize; // in bits
        let mut bit = |n: u32| {
            let mut v = 0;
            for k in 0..n {
                v |= ((src[pos / 8] >> (pos % 8)) & 1) as u32 * (1 << k);
                pos += 1;
            }
            v
        };
        let mut out: Vec<u8> = Vec::new();
        loop {
            let last = bit(1);
            assert_eq!(bit(2), 1, "not a fixed block");
            loop {
                // Read the code MSB first, 7 bits then up to 9.
                let mut code = 0;
                let mut len = 0;
                let sym = loop {
                    code = code << 1 | bit(1);
                    len += 1;
                    match (len, code) {
                        (7, 0..=0x17) => break code + 256,
                        (8, 0x30..=0xbf) => break code - 0x30,
                        (8, 0xc0..=0xc7) => break code - 0xc0 + 280,
                        (9, 0x190..=0x1ff) => break code - 0x190 + 144,
                        _ => assert!(len < 9, "bad code"),
                    }
                };
                match sym {
                    0..=255 => out.push(sym as u8),
                    256 => break,
                    _ => {
                        let l = (sym - 257) as usize;
                        let len = LEN_BASE[l] as usize + bit(LEN_EXTRA[l] as u32) as usize;
                        let mut d = 0;
                        for _ in 0..5 {
                            d = d << 1 | bit(1);
                        }
                        let dist =
                            DIST_BASE[d as usize] as usize + bit(DIST_EXTRA[d as usize] as u32) as usize;
                        for _ in 0..len {
                            out.push(out[out.len() - dist]);
                        }
                    }
                }
            }
            if last == 1 {
                return out;
            }
        }
    }

    fn round_trip(data: &[u8], hint: usize) -> usize {
        let z = compress(data, hint);
        assert_eq!(&z[..2], &[0x78, 0x01]);
        assert_eq!(u16::from_be_bytes([z[0], z[1]]) % 31, 0, "header check bits");
        let body = &z[2..z.len() - 4];
        assert_eq!(inflate_fixed(body), data);
        assert_eq!(&z[z.len() - 4..], &adler32(data).to_be_bytes());
        z.len()
    }

    #[test]
    fn adler_known_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn round_trips() {
        round_trip(b"", 0);
        round_trip(b"a", 0);
        round_trip(b"abcabcabcabcabcabcabcabcabc and more text, more text", 0);
        // Every byte value, long runs, and matches far back in the window.
        let mut v: Vec<u8> = (0..=255).collect();
        v.extend(std::iter::repeat_n(7u8, 1000));
        let mut x = 1u32;
        let noise: Vec<u8> = (0..40000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 24) as u8
            })
            .collect();
        v.extend(&noise);
        v.extend(&noise[..20000]);
        round_trip(&v, 0);
    }

    #[test]
    fn rows_scaled_up_compress_well() {
        // 64 random pixels per row, each repeated 4 times across and the row 4 times down.
        let (w, k) = (64, 4);
        let mut x = 7u32;
        let mut data = Vec::new();
        for _ in 0..16 {
            let row: Vec<u8> = (0..w * 3)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 24) as u8
                })
                .collect();
            let wide: Vec<u8> = row.chunks(3).flat_map(|p| p.repeat(k)).collect();
            for _ in 0..k {
                data.extend(&wide);
            }
        }
        let n = round_trip(&data, w * k * 3);
        assert!(n * 6 < data.len(), "{n} of {}", data.len());
    }
}
