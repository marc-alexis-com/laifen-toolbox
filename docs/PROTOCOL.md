# Laifen Wave (LFTB01-P) control protocol

Wire-level description of the Bluetooth LE protocol of the **Laifen Wave** electric toothbrush
(retail model `LFTB01-P`, product code `T91` inside the vendor app), as implemented by `laifen`.
It was worked out for interoperability, from the behaviour of the vendor's Android app (Laifen
3.3.0) and of the device itself, on a unit owned by the author.

**Scope.** The device tests were made on one LFTB01-P running firmware `V2.0.8.Y44.303`. The
vendor app gives this brush a protocol of its own: the other Laifen brushes speak a different,
longer one, not covered here except for a short appendix.

Each statement carries a status:

- **confirmed**: observed on the device (request sent, answer read, effect read back);
- **app**: how the vendor app builds or parses it, not exercised on the device;
- **inferred**: deduced, with the reasoning given.

Bytes are in hexadecimal. In request templates, literal bytes are lower case, fields upper case,
and `X` is the checksum.

## 1. Transport

| Fact | Status |
|---|---|
| Bluetooth LE (GATT), service `0000ff01-0000-1000-8000-00805f9b34fb`, **one characteristic** `0000ff02-0000-1000-8000-00805f9b34fb` for both directions: commands are written to it, and everything the brush sends arrives as a notification on it. | confirmed |
| Commands are written **with response** (ATT Write Request), as the vendor app does; every write of the device tests was made this way. `0000ff02` also lists write-without-response: never tried. | confirmed |
| One frame per notification. The app reassembles nothing; the 25-byte state frames arrived whole, each in a single notification. | app; confirmed (25 bytes) |
| MTU: the app never requests one. Under BlueZ the 25-byte notifications arrived whole, so the effective ATT MTU is at least 28. Who negotiated it was not looked at. | confirmed |
| No pairing, bonding, encryption or authentication: connecting, subscribing, reading and writing all worked without. The app has none on this path either. | confirmed |
| As soon as notifications are on, the brush sends its firmware version (`0x01`) unprompted, before any write. | confirmed |
| The brush does not advertise all the time: nothing was seen during two 60-minute watches; it showed up within 40 s, then within about 10 s, each time its owner was asked to wake it. A press on its button wakes it; `laifen` suggests one. | confirmed |
| Left alone while connected, the brush dropped the connection 71 s after the last change made at its button (60 s after the repeated state of §3.4), then no longer showed up: back to sleep. The computer sent nothing in between; whether reads keep it awake was not tried. | confirmed (once) |
| While the phone holds the connection, the brush is invisible to the computer: the vendor app can keep its connection open in the background, and a connected brush is not expected to advertise. Not tried. | inferred |
| The vendor app expects a notification within 5 s of each write, and disconnects after three misses in a row. | app |

**Advertising**, on the unit tested:

| Field | Seen | Status |
|---|---|---|
| Name | `LFTB01-P-xxxx`, the four hex digits being the last two bytes of the address | confirmed (one unit) |
| Address | `C0:00:00:00:xx:xx`: four of its six bytes are `c0 00 00 00` | confirmed (one unit) |
| Manufacturer data | company identifier `0xAA00`, payload `xx xx 00 00 00 c0`: the address, byte-reversed | confirmed |
| Service UUIDs | none: a scan must look for the name | confirmed |

Whether other units share the `C0:00:00:00` part is unknown. The vendor app recognises the brush
by name prefix only, without reading the advertising data, and knows two more names for a brush
in firmware-update mode, `PPlusOTA` and `LFTB-S-01-OTA` (**app**).

**GATT table**, as read from the brush (**confirmed**):

```
1800  Generic Access
1801  Generic Attribute (2a05 Service Changed, indicate)
180a  Device Information, 9 characteristics, read
ff01  the protocol
      ff02  read, write, write-without-response, notify   handle 0x0026
      ff10  read                                          handle 0x002a
5833ff01-9b8b-5191-6142-22a4536ef123  Phyplus firmware update
      5833ff02-…  write;  5833ff03-…  notify
```

- Device Information holds SDK placeholders, not product data: each text characteristic contains
  its own name ("Manufacturer Name", "Model Number", "Serial Number"…), the IEEE 11073 data is
  `fe 00` + "experimental", the System ID is made from the address, and the PnP ID is
  `01 04 05 00 00 10 01` (SIG source, vendor `0x0504`, product `0x0000`, version `0x0110`). Only
  the name and the version answer identify the model. **confirmed**. These defaults look like the
  examples of Texas Instruments' BLE stack, from which the Phyplus SDK would derive; that `0x0504`
  is Phyplus's identifier was not checked (**inferred**).
- `0000ff10` reads `01 02 03 04 05 06 07 65 20 52` and the app never refers to it: an SDK example
  left in, probably (**inferred**).
- The Phyplus update service sits next to the protocol's, so the update stack the app uses for
  this model does target this brush (**confirmed**). `ff01`/`ff02` is also that SDK's default
  "app" service: the chip is probably a Phyplus PHY62xx (**inferred**). `laifen` never touches the
  update service (§6).

## 2. Frame

```
offset  size  field
0       1     0xAA header
1       1     command
2       1     kind: 00 read, 01 write (computer to brush); 02 (brush to computer)
3       1     payload length n
4       n     payload
4+n     1     XOR of bytes 0..3+n
```

| Fact | Status |
|---|---|
| The checksum is a plain XOR of every previous byte, header included; no CRC. Right on all 53 frames received. | confirmed |
| `kind` from the computer: `00` for reads, which carry no payload, `01` for writes. The app never names this byte; all 18 of its command codes follow that pattern. | app (values); inferred (meaning) |
| `kind` from the brush: `02` in every frame, answers, acknowledgements and unprompted reports alike (53 frames). | confirmed |
| A 4-byte header, without counter or sequence number (the other Laifen brushes use 6 bytes, appendix). Every frame exchanged with the brush fits this layout, length byte included. | confirmed |
| The vendor app checks nothing it receives (header, `kind`, length, checksum) and dispatches on the command byte alone. `laifen` checks the header, the length and the checksum, and drops any other frame. | app |
| The vendor app picks this protocol from the BLE name (`LFTB01-P`, or the two update-mode names), never from the content of the frames. | app |

Examples, all exchanged with the brush (**confirmed**; the state is decoded in §4, the version in
§5):

```
-> aa 0a 00 00 a0              read the state: aa ^ 0a ^ 00 ^ 00 = a0
<- aa 0a 02 14 00 01 0a 07 0a 0a 0a 04 0a 0a 14 01 01 00 5d 01 00 78 01 00 85
-> aa 01 00 00 ab              read the version
<- aa 01 02 09 02 00 08 59 04 04 03 00 03 f3
-> aa 05 01 02 00 96 3a        set the brushing time to 150 s
<- aa 05 02 01 00 ac           accepted
```

## 3. Commands

The vendor app knows 18 command codes and sends 14 of them; the brush sends two more of its own.
On connecting, the app reads the state 500 ms after subscribing, and the version when its control
screen opens; it never sets the clock, and reads the history only on a hidden gesture (**app**).

### 3.1 Computer to brush

`VV` is a one-byte switch: `01` on, `00` off.

| Code | Request | What it does | Answer | Status | `laifen` |
|---|---|---|---|---|---|
| `0x01` | `aa 01 00 00 ab` | read the firmware version | `0x01`, 9 bytes (§5) | confirmed | `status` |
| `0x02` | `aa 02 01 06 YY MO DD HH MI SS X` | set the date; never called by the app | the app ignores a `0x02` | app | refused |
| `0x03` | `aa 03 00 00 a9` | read the date; never sent by the app | `0x03`, year to second, 6 bytes | app | refused |
| `0x04` | `aa 04 01 04 MO ST RA SP X` | make mode `MO` active and set its strength, range and speed (§3.5) | acknowledgement, then the state | confirmed | `mode set`, `mode tune` |
| `0x05` | `aa 05 01 02 HI LO X` | brushing time in seconds, big-endian, one value for every mode | acknowledgement | confirmed | `duration set` |
| `0x06` | `aa 06 01 01 VV X` | head calibration: `01` enter, `03` start, `00` cancel | the app takes any `0x06` as success | app | refused |
| `0x07` | `aa 07 01 01 VV X` | airplane mode | the app ignores a `0x07` | app | refused |
| `0x08` | `aa 08 00 00 a2` | read the battery; never sent by the app | `0x08`, 1 byte | app | refused |
| `0x09` | `aa 09 00 00 a3` | read the brushing history | one `0x09` per record, undated | app | refused |
| `0x0A` | `aa 0a 00 00 a0` | read the whole state | `0x0A` (§4) | confirmed | every command |
| `0x0B` | `aa 0b 01 01 VV X` | 30-second reminder | acknowledgement | confirmed | `reminder` |
| `0x0C` | `aa 0c 01 01 MO X` | restore mode `MO` (0–3) to its defaults | the app ignores a `0x0C` | app | refused |
| `0x0D` | `aa 0d 01 01 01 a6` | factory reset | `0x0D`, unused by the app | app | refused |
| `0x0E` | `aa 0e 01 01 VV X` | high frequency, the fourth mode | acknowledgement, then the state | confirmed | `high-freq` |
| `0x0F` | `aa 0f 01 01 VV X` | motor: `01` start, `00` stop | `0x0F` with a status, then the state | confirmed | `start`, `stop` |
| `0x10` | `aa 10 00 00 ba` | read a device ID; never sent by the app | `0x10`; the app derives a serial number from it | app | refused |
| `0x14` | `aa 14 01 01 VV X` | emergency shutdown | the app ignores a `0x14` | app | refused |
| `0x16` | `aa 16 01 01 VV X` | `01` deep clean, `00` gentle clean | the app ignores a `0x16` | app | refused |

### 3.2 Brush to computer, unasked

| Code | Meaning | Status |
|---|---|---|
| `0x01` | version, sent as soon as notifications are on | confirmed |
| `0x0A` | state: after `0x04`, `0x0E` and `0x0F`, after every start, stop or mode change made at the brush's button, and again about 10 s after the last change (§3.4) | confirmed |
| `0x11` | fault code, 1 byte. Non-zero: the app blocks deep/gentle clean and firmware updates; it resets the code to 0 on each connection | app, never seen |
| `0x17` | charging state, 1 byte, as in the state (§4) | app, never seen |

### 3.3 Acknowledgements

Each write tried (`0x04`, `0x05`, `0x0B`, `0x0E`, `0x0F`) is answered by `aa CC 02 01 SS X`, the
command code `CC` and a status `SS`, `00` meaning accepted: `aa 04 02 01 00 ad`,
`aa 05 02 01 00 ac`, `aa 0b 02 01 00 a2`, `aa 0e 02 01 00 a7`, `aa 0f 02 01 00 a6`. **confirmed**

No value was ever refused, so a non-zero status was never seen. For `0x0F` the app shows
`01` "Toothbrush is charging and cannot be used", `02` "Toothbrush malfunction", `03` "Low
battery" (**app**). For the other writes the app reads nothing from the answer: to it, success
means the GATT write went through. `laifen` requires status `00` after every write, and gives the
app's reason when `0x0F` is refused.

### 3.4 Pushed state

- After `0x04`, `0x0E` and `0x0F`, the brush pushes its state on its own, sometimes twice,
  sometimes one write late: once, a state still showing Mode 3 arrived after the order to go back
  to Mode 2, just before its acknowledgement. After `0x05` and `0x0B`, nothing. **confirmed**
- The button sends nothing of its own: every change made with it is pushed at once as a full
  state, which differs from the previous one by a single byte, byte 19 when the motor starts or
  stops, byte 0 when the mode changes. In 90 s of button presses the brush pushed 42 states and
  no other frame (**confirmed**). At the button, modes cycle Mode 1 → Mode 2 → Mode 3 → high
  frequency → Mode 1, high frequency being on, and the mode can change while the motor runs
  (**confirmed**).
- About 10 s after the last change, the brush pushes the same state again, unchanged: after
  10.1 s and 10.9 s, seen twice (**confirmed**); why is not known. Once, an unchanged state also
  came 0.9 s before the motor started. The two unasked states seen earlier, 10 s apart and both
  with the active mode gone from Mode 1 to Mode 2, fit this: a change at the button, then its
  repeat (**inferred**).
- A client must therefore read the state back rather than trust the last one pushed. `laifen`
  waits 500 ms after a write, reads the state, and reads it once more 800 ms later if the change
  is not there yet.

### 3.5 Modes and settings

- `0x04` takes `[mode, strength, range, speed]`; modes `00`–`02` are Mode 1–3, `03` is high
  frequency. Writing a mode **also makes it the active mode**: writing Mode 3's current settings
  switched the brush to Mode 3 (**confirmed**). To change another mode's settings, write them,
  then write the active mode back with its own: that is what `laifen mode tune` does.
- Mode 2's strength, range and speed were each written (10 → 9 → 10), and each write changed only
  its own field, in `0x04` as in the state (§4). **confirmed**. Examples:
  `aa 04 01 04 01 0a 0a 09 a3` (Mode 2, speed 9), `aa 04 01 04 02 04 0a 0a ad` (switch to Mode 3,
  keeping 4/10/10).
- Ranges, from the app's sliders: strength 1–10 (11–20 for high frequency), range 1–10, speed 1–10
  (**app**). `laifen` sends nothing outside them.
- `0x05`: the app offers 1 to 5 minutes in 30-second steps (60–300 s), and so does `laifen`.
  `aa 05 01 02 00 78 d4` = 120 s. **confirmed**
- The high-frequency mode exists only while `0x0E` is on: the app shows its tab only then, and
  turning high frequency off from that tab also switches to Mode 3 with a `0x04` (**app**).
  `laifen` refuses to select or tune it while it is off, and to turn it off while it is the active
  mode; what the brush itself does then was not tried.
- After `0x0C`, the app shows its own defaults without reading the state: strength/range/speed
  1/3/1, 1/6/1 and 1/10/1 for Modes 1–3, 11/1/1 for high frequency (**app**). That the brush
  applies the same is **inferred**.

### 3.6 Checks made by the vendor app

The app refuses some changes on its own side (**app**); whether the brush refuses them too is not
known.

- Airplane mode on: every other change (motor, reminder, high frequency, brushing time, modes,
  restore, deep/gentle clean). `laifen` refuses every change too, except `stop`.
- Charging (state byte 21, §4): motor, airplane mode, reminder, high frequency. The 20-byte state of
  this firmware does not carry the charging state, so `laifen` cannot check it.
- Emergency shutdown on: motor and deep/gentle clean. Non-zero fault code: deep/gentle clean.
- Battery under 10: the app shows the brush as stopped, with "Low battery".

## 4. State (`0x0A`)

Four layouts, told apart by length: 15, 20, 21 and 23 bytes. The app knows 15, 21 and 23; **the
brush tested sends 20**, a length absent from the app's code (**confirmed**). It is the 21-byte
layout without its last byte, emergency shutdown. Bytes are numbered within the payload (add 4 for
the frame); the "Seen" column decodes the frame of §2.

| Byte | Field | Values | Seen | Status |
|---|---|---|---|---|
| 0 | active mode | `00`–`02` Mode 1–3, `03` high frequency | `00` | confirmed (0–2) |
| 1–3 | Mode 1: strength, range, speed | 1–10 each | `01 0a 07` | app; fits |
| 4–6 | Mode 2: strength, range, speed | 1–10 each | `0a 0a 0a` | confirmed |
| 7–9 | Mode 3: strength, range, speed | 1–10 each | `04 0a 0a` | app; fits |
| 10–12 | high frequency: strength, range, speed | 11–20, 1–10, 1–10 | `14 01 01` | app; fits |
| 13 | airplane mode | boolean | `00` | app |
| 14 | battery | percent, probably | `5d` = 93 | confirmed (unit inferred) |
| 15 | 30-second reminder | boolean | `01` | confirmed |
| 16–17 | brushing time, seconds, big-endian | 60–300, steps of 30 | `00 78` = 120 | confirmed |
| 18 | high frequency on (`0x0E`) | boolean | `01` | confirmed |
| 19 | motor running | boolean | `00` | confirmed |
| 20 | emergency shutdown on (21 and 23 bytes) | boolean | | app |
| 21 | charging state (23 bytes) | `01` charging; `00` and `02` (unexplained) hide the app's charging icon | | app |
| 22 | deep clean, else gentle (23 bytes) | boolean | | app |

*app; fits*: the field is placed by the app's layout, and the value read on the device fits it,
but that byte was never changed in a test.

- **15 bytes** (**app**): byte 0 active mode (0–2), 1–9 Modes 1–3, 10 airplane mode, 11 battery,
  12 reminder, 13–14 brushing time. No high-frequency settings, and none of the fields of bytes
  18–22 above.
- The app picks the layout from the payload length (15), or else from the **frame** length:
  27 bytes or more is read as 23, anything shorter as 21. A 20-byte payload is thus read as 21
  bytes, the 21st being the checksum, which the app shows as "emergency shutdown" (on only if the
  checksum is `01`). Harmless with this firmware, on which the app refuses emergency shutdown anyway
  (`NotifyOldTbData.java:601-605`, **app**). When the frame is under 27 bytes, the app also forces
  "deep clean" to on.
- A boolean is true only for exactly `01` (**app**); `laifen` does the same.
- Brushing time: decode `hi × 256 + lo`, the inverse of what the app sends (**inferred**). The
  app's own decoder adds `hi + 255` instead of `hi × 256`, right only up to 511 s (512 would read
  257); harmless, since it never offers more than 300.
- Battery: a percentage, probably, since the app draws it as a 0–100 bar with thresholds at 20, 50,
  70 and 90 (**inferred**). It read 93, 92 a quarter of an hour later, then 91 after 5 s of motor
  running: the reading probably sags under load (**inferred**).
- Emergency shutdown, deep clean and the charging state are not in the 20-byte layout: on this
  firmware they cannot be read back.

## 5. Version (`0x01`)

- **9 bytes** `b0`…`b8`, printed `V<b0>.<b1>.<b2>.<b3><b4><b5>.<b6><b7><b8>`, each byte as a
  signed decimal number except `b3`, an ASCII letter: `02 00 08 59 04 04 03 00 03` →
  `V2.0.8.Y44.303`. **confirmed** (the brush sent these 9 bytes; the rendering is the app's).
- **19 bytes**: a longer format with more bytes rendered as letters; its exact rendering could not
  be established from the decompiled app. **app**, unverified. `laifen` prints any length other
  than 9 in hex.
- The app adapts to the version (**app**): on 2.0.7 and 2.0.8 it refuses emergency shutdown
  ("Firmware not supported"); it hides deep/gentle clean when the version contains `2.0.5`,
  `2.0.7`, `2.0.8`, `2.0.9`, `V1.` or `0.2.5`; and it sends the version to the vendor's server to
  look for an update.

## 6. Commands this tool never sends

`laifen` only sends what was sent to a real brush, read back and undone: the reads `0x01` and
`0x0A`, the writes `0x04`, `0x05`, `0x0B`, `0x0E` and `0x0F` (`src/cmd.rs`). Anything else is
refused before a frame is built, a read code cannot go out as a write or the reverse, and there is
no override.

| Code | Why |
|---|---|
| `0x0D` | Factory reset. Afterwards the app waits for the brush to disconnect (except on 2.0.7 and 2.0.8), then reconnects: a reboot, probably (**inferred**). |
| `0x06` | Brush-head calibration: a physical procedure the app walks the user through, which "cannot be exited once started". |
| `0x02` | Set the date: never used by the app, and its year encoding is unknown (the year truncated to a byte, or the year minus 2000: no caller says). |
| `0x07` | Airplane mode: it may turn the brush's radio off (**inferred**, not tried), leaving the computer no way back, and how to leave it on the brush is unknown. |
| `0x14` | Emergency shutdown ("used to force-shutdown the toothbrush"): untested, refused by the app on 2.0.7 and 2.0.8, and absent from the 20-byte state. |
| `0x16` | Deep / gentle clean: untested, hidden by the app on 2.0.8 (§5), and absent from the 20-byte state. |
| `0x0C` | Restore a mode's defaults: overwrites the mode with values the brush chooses; the app shows its own and never reads them back. |
| `0x09` | Brushing history: reading it may clear it on the brush before the vendor app collects it. |
| `0x03`, `0x08`, `0x10` | Reads the app never sends: not tested. |
| `01 02` | Written raw to `0000ff02`, the Phyplus update SDK's command that switches the chip into update mode (`OTACore.java:494`, **app**). Every frame `laifen` writes starts with `aa`. |
| `5833ff01-…` | The Phyplus update service: `laifen` only ever looks up `0000ff02`, and does not connect to a device named `PPlusOTA` or `LFTB-S-01-OTA` (a brush in update mode). |

## 7. Unknowns

1. What the brush checks in what it receives: `kind`, length, checksum. Nothing malformed was sent
   to find out, and nothing will be.
2. Who negotiates the MTU, BlueZ or the brush.
3. Which gesture on the button does what: the pushed states show the effects, not the presses.
   Why the state is pushed again 10 s after the last change. The `0x11` and `0x17` reports,
   never seen.
4. Non-zero acknowledgement statuses, the charging state `02`, the list of fault codes.
5. What airplane mode and emergency shutdown really do, whether airplane mode turns the radio off,
   and how to undo either from the brush.
6. When high frequency was turned back on, a pushed state showed the motor running (byte 19 =
   `01`), back to `00` a second later. The owner felt nothing, but may not have been holding the
   brush.
7. The clock: whether the brush has one, the year encoding of `0x02`, and whether `0x02`, `0x03`,
   `0x08` and `0x10` are supported at all.
8. The history: two of the record's fields, which firmware sends the short or the long record, in
   what order, and whether reading the history clears it.
9. Calibration: whether the brush answers `0x06` on entry or only at the end.
10. The 19-byte version format, and what each firmware (2.0.5, 2.0.7, 2.0.8, 2.0.9, V1.x, 0.2.5)
    supports.
11. The values the brush applies after `0x0C` and `0x0D`.
12. Whether anything but the button wakes the brush (its stand, picking it up), and whether reads
    keep it awake past the 71 s seen idle; whether a brush connected to the phone is
    really invisible; whether it can be reached from a new connection while its motor runs (the
    stop of the tests went over the connection that started it); whether its button stops a motor
    started remotely.

## Appendix: the other Laifen brushes

The rest of the Laifen lineup speaks another protocol, on the same service and characteristic:
the SE family (TB31, advertising as `LFTB01-SE`, with TB32, TB51 and T91S, whose names are not
established), the TB92 (`LFTB02-S`, a prefix other models share, so the app asks the brush for its
model), and the TC51 kids' brush (`LFTB01-C`) with its base (`TC51BB`). Its frames are longer:

```
AA | (function << 14) | command, 16-bit BE | counter | length, 16-bit BE | payload | XOR
```

with function 0 write, 1 read, 2 report and 3 answer, a counter left at 0 outside firmware
updates, and the same XOR of every previous byte. The vendor app picks one protocol or the other
from the BLE name alone.

That protocol is not described here: it was read in the vendor app only, none of it was
exercised on a device, and `laifen` does not support these brushes.
