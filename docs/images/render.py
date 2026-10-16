"""Renders the README images of laifen-toolbox (PNG, drawn at 2x).

    python3 render.py OUT_DIR IOSEVKA_REGULAR.ttf IOSEVKA_BOLD.ttf POPPINS_DIR NAME=FILE.ansi...

Needs Pillow. Fonts: Iosevka (https://github.com/be5invis/Iosevka) for the terminal
screenshots, Poppins (https://github.com/itfoundry/Poppins, SIL OFL; POPPINS_DIR holds
Poppins-Light/Regular/Medium/SemiBold.ttf) for the banner: the geometric sans the vendor's
own site falls back to.
Each NAME=FILE.ansi pair renders FILE.ansi to OUT_DIR/NAME.png. The pictures come from the
tool's own renderer (`LAIFEN_PREVIEW_DIR=DIR cargo test --release readme_pictures -- --ignored`),
read from $PICTURES: `spin/` (the brush turning, for the banner), `tui.png` and `brushing/`
(the 3D view behind the window of `laifen tui`). The window itself is a real capture of
`laifen tui --blocks` (`tmux capture-pane -e -p`, 150 x 42), given as $TUI_WINDOW (one file) and
$BRUSHING_WINDOWS (a directory of frame-*.ansi). The other inputs are real terminal output
captured with `tmux capture-pane -e -p`, one-shot commands preceded by their own prompt line.

Style: the vendor's own look, as on its site and in its app: a light-grey studio, near-black
text (#151515), neutral greys, rounded pills, and nothing else; a #151515 version for dark
pages. Terminal shots are light cards. The 3D brush is the tool's own renderer, pixels and all.
Adapted from nothing-toolbox's render.py (same author).
"""
import math
import os
import re
import sys

from PIL import Image, ImageColor, ImageDraw, ImageFilter, ImageFont

OUT, IOS_R, IOS_B, POPPINS = sys.argv[1:5]
SHOTS = [arg.split("=", 1) for arg in sys.argv[5:]]
PICTURES = os.environ["PICTURES"]


def brush_frames():
    """The brush turning once: RGBA frames, all cropped to its widest extent."""
    d = os.path.join(PICTURES, "spin")
    files = sorted(os.path.join(d, f) for f in os.listdir(d) if f.startswith("frame-"))
    pics = [Image.open(f).convert("RGBA") for f in files]
    boxes = [p.getbbox() or (0, 0, p.width, p.height) for p in pics]
    box = (min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes))
    return [p.crop(box) for p in pics]


TURN_MS = 6000  # one turn of the brush
FLOAT = 3  # how far it floats up and down, in its own (coarse) pixels


def turning(frames, img, x0, y0, w, h):
    """The banner or diagram `img`, once per headset frame, with the frame fitted into the
    w x h box at (x0, y0). The frames are pixel art: enlarged by a whole factor, so their pixels
    stay sharp squares. An animation also floats once up and down per turn, by whole pixels."""
    out = []
    k = max(1, min((w * K) // frames[0].width, (h * K - 2 * FLOAT * K) // frames[0].height))
    for i, pic in enumerate(frames):
        pic = pic.resize((pic.width * k, pic.height * k), Image.NEAREST)
        lift = round(FLOAT * math.sin(math.tau * i / len(frames))) * k if len(frames) > 1 else 0
        frame = img.copy()
        frame.paste(pic, (x0 * K + (w * K - pic.width) // 2, y0 * K + (h * K - pic.height) // 2 - lift), pic)
        out.append(frame)
    return out


def save(frames, path):
    """One PNG, or an animated PNG (the headset turning once in about 4.3 s)."""
    if len(frames) == 1:
        frames[0].save(path, optimize=True)
        return
    # One 256-colour palette for every frame (the page is grey, the headset white): a few times
    # smaller than full colour, with no visible difference.
    sample = Image.new("RGB", (frames[0].width, frames[0].height * 4))
    for i in range(4):
        sample.paste(frames[i * len(frames) // 4], (0, frames[0].height * i))
    palette = sample.quantize(colors=256, method=Image.Quantize.MEDIANCUT)
    frames = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in frames]
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=TURN_MS / len(frames), loop=0,
                   optimize=True)


K = 2  # supersampling: every logical pixel is K device pixels

# --- Design tokens: the vendor's look ----------------------------------------------------
THEMES = {
    # The vendor's site: white and #f5f5f5, text #151515, greys #585858 / #888888, rules #e2e2e2.
    "light": dict(top="#f7f7f8", bottom="#e4e5e7", ink="#151515", body="#585858", muted="#888888",
                  hair="#e2e2e2", pill="#ffffff", shadow=(0, 0, 0, 46)),
    # Its footer and dark sections: #151515 and #1c1c1c.
    "dark": dict(top="#232325", bottom="#151515", ink="#ffffff", body="#b8b8bc", muted="#888888",
                 hair="#333336", pill="#1c1c1e", shadow=(0, 0, 0, 110)),
}


def mono(size, bold=False):
    return ImageFont.truetype(IOS_B if bold else IOS_R, size * K)


def poppins(size, weight="Regular"):
    return ImageFont.truetype(os.path.join(POPPINS, f"Poppins-{weight}.ttf"), size * K)


def spaced(draw, xy, text, font, fill, tracking=0.0):
    """Draws text with extra letter spacing (in logical px)."""
    x, y = xy
    for ch in text:
        draw.text((x, y), ch, font=font, fill=fill)
        x += draw.textlength(ch, font=font) + tracking * K
    return x


def text_width(draw, text, font, tracking=0.0):
    return sum(draw.textlength(c, font=font) for c in text) + tracking * K * max(len(text) - 1, 0)


def hero(theme):
    t = THEMES[theme]
    W, H = 960, 400
    img = Image.new("RGB", (W * K, H * K), t["top"])
    d = ImageDraw.Draw(img)
    # A studio backdrop: a soft vertical gradient, as behind the vendor's product shots.
    top, bottom = ImageColor.getrgb(t["top"]), ImageColor.getrgb(t["bottom"])
    for y in range(H * K):
        f = y / (H * K - 1)
        d.line([(0, y), (W * K, y)], fill=tuple(round(a + (b - a) * f) for a, b in zip(top, bottom)))

    x = 64
    cap = poppins(11, "Medium")
    chip = "UNOFFICIAL LINUX CLIENT"
    cw = text_width(d, chip, cap, 1.4) / K
    d.rounded_rectangle([x * K, 70 * K, (x + cw + 28) * K, 96 * K], radius=13 * K, fill=t["pill"], outline=t["hair"], width=K)
    spaced(d, ((x + 14) * K, 75 * K), chip, cap, t["muted"], 1.4)

    d.text((x * K, 112 * K), "laifen-toolbox", font=poppins(50, "SemiBold"), fill=t["ink"])
    body = poppins(19, "Light")
    d.text((x * K, 192 * K), "Control your Laifen Wave", font=body, fill=t["body"])
    d.text((x * K, 220 * K), "from a Linux terminal.", font=body, fill=t["body"])

    px = x
    tag = poppins(13, "Medium")
    for label in ["LFTB01-P", "Bluetooth LE", "No app, no account", "Rust"]:
        w = text_width(d, label, tag) / K
        d.rounded_rectangle([px * K, 288 * K, (px + w + 30) * K, 320 * K], radius=16 * K, fill=t["pill"], outline=t["hair"], width=K)
        d.text(((px + 15) * K, 294 * K), label, font=tag, fill=t["ink"])
        px += w + 40

    # The brush on the right, over a soft shadow on the floor of the studio.
    box = (600, 28, 300, 344)
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    cx, cy = (box[0] + box[2] / 2) * K, (box[1] + box[3] - 14) * K
    sd.ellipse([cx - 70 * K, cy - 9 * K, cx + 70 * K, cy + 9 * K], fill=t["shadow"])
    shadow = shadow.filter(ImageFilter.GaussianBlur(9 * K))
    img = Image.alpha_composite(img.convert("RGBA"), shadow).convert("RGB")
    save(turning(BRUSH_FRAMES, img, *box), f"{OUT}/hero-{theme}.png")


# --- Terminal screenshots, kitty-like -------------------------------------------------------
AYU = {
    # A light terminal in the vendor's colours: #f5f5f5 paper, #151515 ink, its greys, and the
    # app's blue and green for the few coloured bits.
    "bg": "#f5f5f5", "fg": "#151515",
    "pal": ["#151515", "#d92d20", "#279a4b", "#b7791f", "#3b82f6", "#8e44ad", "#0e7490", "#585858",
            "#888888", "#e5484d", "#279a4b", "#b7791f", "#3b82f6", "#8e44ad", "#0e7490", "#151515"],
}
SGR = re.compile(r"\x1b\[([0-9;]*)m")
OTHER = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def parse_ansi(text):
    """Returns lines of (char, color, bold, background or None)."""
    lines = []
    for raw in text.split("\n"):
        out, color, bold, bg, pos = [], AYU["fg"], False, None, 0
        raw = OTHER.sub(lambda m: m.group(0) if m.group(0).endswith("m") else "", raw)
        for m in SGR.finditer(raw):
            out += [(c, color, bold, bg) for c in raw[pos:m.start()]]
            pos = m.end()
            codes = [int(c) for c in m.group(1).split(";") if c] or [0]
            i = 0
            while i < len(codes):
                c = codes[i]
                if c == 0:
                    color, bold, bg = AYU["fg"], False, None
                elif 40 <= c <= 47:
                    bg = AYU["pal"][c - 40]
                elif c == 48 and i + 4 < len(codes) and codes[i + 1] == 2:
                    bg = "#%02x%02x%02x" % tuple(codes[i + 2:i + 5])
                    i += 4
                elif c == 49:
                    bg = None
                elif c == 1:
                    bold = True
                elif c == 22:
                    bold = False
                elif c == 39:
                    color = AYU["fg"]
                elif 30 <= c <= 37:
                    color = AYU["pal"][c - 30]
                elif 90 <= c <= 97:
                    color = AYU["pal"][c - 90 + 8]
                elif c == 38 and i + 2 < len(codes) and codes[i + 1] == 5:
                    color = AYU["pal"][codes[i + 2]] if codes[i + 2] < 16 else AYU["fg"]
                    i += 2
                elif c == 38 and i + 4 < len(codes) and codes[i + 1] == 2:
                    color = "#%02x%02x%02x" % tuple(codes[i + 2:i + 5])
                    i += 4
                i += 1
        out += [(c, color, bold, bg) for c in raw[pos:]]
        lines.append(out)
    while lines and not any(ch.strip() for ch, _, _, _ in lines[-1]):
        lines.pop()
    return lines


BRAILLE_DOTS = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
                (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)]


def braille(d, x, y, w, h, bits, color):
    """Draws a braille cell as kitty does: 2x4 round dots filling the whole cell, so that
    curves drawn with them look continuous (font braille glyphs are much smaller)."""
    r = min(w / 2, h / 4) * 0.36
    for col, row, bit in BRAILLE_DOTS:
        if bits & bit:
            cx, cy = x + (col + 0.5) * w / 2, y + (row + 0.5) * h / 4
            d.ellipse([(cx - r) * K, (cy - r) * K, (cx + r) * K, (cy + r) * K], fill=color)


def terminal(src, name, cols=None):
    """A light card with round corners, transparent around them."""
    body = terminal_image(src, cols)
    card = Image.new("RGBA", body.size, (0, 0, 0, 0))
    mask = Image.new("L", body.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, body.width - 1, body.height - 1], radius=14 * K, fill=255)
    card.paste(body, (0, 0), mask)
    ImageDraw.Draw(card).rounded_rectangle([0, 0, body.width - 1, body.height - 1], radius=14 * K,
                                           outline="#e2e2e2", width=K)
    card.save(f"{OUT}/{name}.png", optimize=True)


def animation(directory, name):
    files = sorted(f for f in os.listdir(directory) if f.startswith("frame-") and f.endswith(".ansi"))
    frames = []
    for f in files:
        img = terminal_image(os.path.join(directory, f))
        img = img.resize((img.width // K, img.height // K), Image.LANCZOS)
        frames.append(img.quantize(colors=128, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE))
    frames[0].save(f"{OUT}/{name}.gif", save_all=True, append_images=frames[1:], duration=105, loop=0,
                   optimize=True, disposal=1)


def terminal_image(src, cols=None):
    text = open(src, encoding="utf-8").read()
    lines = parse_ansi(text)
    size = 15
    reg, bld = mono(size), mono(size, bold=True)
    cell = reg.getlength("M") / K  # Iosevka: exactly 0.5 em
    lh = round(size * 1.3)
    ncols = cols or max(len(l) for l in lines)
    pad_x, pad_y = 26, 22
    W = int(pad_x * 2 + ncols * cell)
    H = int(pad_y * 2 + len(lines) * lh)
    img = Image.new("RGB", (W * K, H * K), AYU["bg"])
    d = ImageDraw.Draw(img)
    for row, line in enumerate(lines):
        for col, (ch, color, bold, bg) in enumerate(line):
            if bg:
                x, y = (pad_x + col * cell) * K, (pad_y + row * lh) * K
                d.rectangle([x, y, x + cell * K, y + lh * K], fill=bg)
            if ch == " ":
                continue
            if ch in "▀▄█":
                # Half blocks fill their half of the cell exactly, as in kitty: the glyph would
                # leave gaps with this line height.
                x, y = (pad_x + col * cell) * K, (pad_y + row * lh) * K
                mid = y + lh * K / 2
                top, bottom = (y, mid) if ch == "▀" else (mid, y + lh * K) if ch == "▄" else (y, y + lh * K)
                d.rectangle([x, top, x + cell * K, bottom], fill=color)
                continue
            if 0x2800 < ord(ch) <= 0x28FF:
                braille(d, pad_x + col * cell, pad_y + row * lh, cell, lh, ord(ch) - 0x2800, color)
                continue
            d.text(((pad_x + col * cell) * K, (pad_y + row * lh) * K), ch, font=bld if bold else reg, fill=color)
    return img


# --- laifen tui: the picture, and the real window over it -----------------------------------
COLS, ROWS = 150, 42


def tui_image(picture, window_src):
    """The terminal as kitty shows `laifen tui`: the picture over the whole grid, the window
    (the right-hand 36 columns of a real capture) drawn over it."""
    lines = parse_ansi(open(window_src, encoding="utf-8").read())
    size = 15
    reg, bld = mono(size), mono(size, bold=True)
    cell = reg.getlength("M") / K
    lh = round(size * 1.3)
    W, H = round(COLS * cell), ROWS * lh
    img = Image.open(picture).convert("RGB").resize((W * K, H * K), Image.NEAREST)
    d = ImageDraw.Draw(img)
    cells = [(row, col, line[col]) for row, line in enumerate(lines[:ROWS])
             for col in range(COLS - 36, min(len(line), COLS))]
    # Backgrounds first, then the glyphs: a glyph wider than its cell must not be cut by the
    # next cell's background. The picture shows through where there is none.
    for row, col, (ch, color, bold, bg) in cells:
        if bg is not None:
            x, y = col * cell * K, row * lh * K
            d.rectangle([x, y, x + cell * K, y + lh * K], fill=bg)
    for row, col, (ch, color, bold, bg) in cells:
        if bg is not None and ch != " ":
            if ch in "▀▄█░":
                x, y = col * cell * K, row * lh * K
                if ch == "░":
                    d.rectangle([x + K, y + lh * K * 0.3, x + (cell - 1) * K, y + lh * K * 0.7], outline=color, width=K)
                else:
                    d.rectangle([x, y + lh * K * 0.2, x + cell * K, y + lh * K * 0.8], fill=color)
                continue
            d.text((col * cell * K, row * lh * K), ch, font=bld if bold else reg, fill=color)
    return img


def tui_shot(name):
    tui_image(os.path.join(PICTURES, "tui.png"), os.environ["TUI_WINDOW"]).save(f"{OUT}/{name}.png", optimize=True)


def tui_animation(name):
    windows = os.environ["BRUSHING_WINDOWS"]
    pics = sorted(f for f in os.listdir(os.path.join(PICTURES, "brushing")) if f.startswith("frame-"))
    wins = sorted(f for f in os.listdir(windows) if f.startswith("frame-"))
    frames = []
    for i, p in enumerate(pics):
        img = tui_image(os.path.join(PICTURES, "brushing", p), os.path.join(windows, wins[i * len(wins) // len(pics)]))
        # Keep the brush and the window, at the size of the other animations.
        img = img.crop((img.width * 22 // 100, 0, img.width, img.height))
        img = img.resize((720, round(720 * img.height / img.width)), Image.LANCZOS)
        frames.append(img)
    sample = Image.new("RGB", (frames[0].width, frames[0].height * 4))
    for i in range(4):
        sample.paste(frames[i * len(frames) // 4], (0, frames[0].height * i))
    palette = sample.quantize(colors=128, method=Image.Quantize.MEDIANCUT)
    frames = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in frames]
    frames[0].save(f"{OUT}/{name}.gif", save_all=True, append_images=frames[1:], duration=100, loop=0, optimize=True)


BRUSH_FRAMES = brush_frames()
for th in THEMES:
    hero(th)
tui_shot("tui")
tui_animation("brushing")
for name, src in SHOTS:
    if os.path.isdir(src):
        animation(src, name)
    else:
        terminal(src, name)
print("ok")
