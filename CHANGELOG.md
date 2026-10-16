# Changelog

## 0.1.1 — 2026-10-16

- `tui`: the window takes the colours of the vendor's app, iOS's system palette in dark mode
  (grey labels, blue accent and selection, green / orange / red states), and sets its own text
  colour: on a light terminal theme its text used to be dark on the dark window.
- `tui`: lights that are off are no longer drawn: as on the real brush, they disappear.
- README: in the vendor's own look, a light studio banner set in Poppins and light terminal
  cards, the 3D brush unchanged.

## 0.1.0 — 2026-10-16

First version. Tested on a Laifen Wave, model LFTB01-P, firmware V2.0.8.Y44.303.

- **Status**: `status` (also `--json`), `battery`: firmware, battery, active mode, every
  mode's settings, brushing time, 30 s reminder, high frequency, airplane mode, brushing or not.
- **Modes**: `mode set` switches among modes 1-3 and high frequency; `mode tune` sets a mode's
  vibration strength, oscillation range and speed without switching to it.
- **Brushing**: `duration` (1 to 5 minutes in 30 s steps), `reminder`, `high-freq`, and `start`
  / `stop` the motor.
- **3D view**: `tui` draws the brush in polished steel (or aluminium, or white), turning on a
  mirror floor, with every setting at hand on the arrow keys. While it brushes, its head swings
  with the mode's range and speed. A real picture in kitty (tmux included), text blocks
  elsewhere; `--demo` simulates a brush.
- **One wake, several commands**: the brush goes back to sleep soon after a disconnection, so
  `shell` runs commands one per line over one connection, typed or piped in.
- **Events and discovery**: `watch` prints what the brush reports; `scan` lists the Laifen
  devices nearby.
- Only commands tested on a real brush can be sent; every write is acknowledged by the brush,
  then read back. BlueZ is reached over D-Bus by a small client of our own: no native
  dependency.
- Verified on an LFTB01-P: every setting changed, read back and restored in one session, the
  motor started and stopped (the owner felt it run); `tui` connected, changed the brushing time
  from the keyboard and back, started and stopped brushing.
