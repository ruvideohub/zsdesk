#!/usr/bin/env python3
"""Draw the suite's icons and write them next to their crates.

Writes `crates/aura-dock/resources/aura-dock.ico` and
`crates/aura-service/resources/auraservice.ico`.

The file manager's own icon (`crates/aura-explorer/resources/aura.ico`) is a golden
macOS-style folder; the panel's icon is the same gold, drawn as the panel itself —
a rounded bar with three app tiles on it — so the two read as one product in Explorer
and in the Start menu.

The service is the same gold drawn as a gear, so the programs read as one family.
`auraservice.exe` had no icon at all before this — it showed up as a blank generic
program in Explorer, in the Start menu and in the uninstall list.

Every size is drawn from scratch at 4x and downsampled, which keeps the 16 px and
20 px variants legible instead of mush. Run it from the repository root:

    python3 scripts/make-icons.py
"""
from PIL import Image, ImageDraw, ImageFilter

GOLD_TOP = (247, 204, 94)
GOLD_BOTTOM = (225, 166, 58)
SHADOW = (183, 126, 32)
TILE = (255, 245, 204)
TILE_EDGE = (238, 215, 148)

SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
BOX = 256          # design canvas; everything below is in these units
SUPERSAMPLE = 4


def gradient(size, top, bottom):
    """Vertical gradient as an RGBA image."""
    image = Image.new("RGBA", (1, size), top + (255,))
    draw = ImageDraw.Draw(image)
    for y in range(size):
        t = y / max(size - 1, 1)
        draw.point(
            (0, y),
            fill=(
                round(top[0] + (bottom[0] - top[0]) * t),
                round(top[1] + (bottom[1] - top[1]) * t),
                round(top[2] + (bottom[2] - top[2]) * t),
                255,
            ),
        )
    return image.resize((size, size))


def rounded_mask(size, box, radius):
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(box, radius=radius, fill=255)
    return mask


def draw_icon(units=BOX, scale=SUPERSAMPLE):
    """One icon at `units` logical pixels, drawn `scale` times larger."""
    size = units * scale
    k = size / BOX

    bar = (18 * k, 92 * k, 238 * k, 188 * k)
    radius = 34 * k

    # Drop shadow: the same bar, nudged down and blurred, so the icon sits on the
    # Explorer background instead of floating.
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (bar[0], bar[1] + 7 * k, bar[2], bar[3] + 9 * k),
        radius=radius,
        fill=SHADOW + (110,),
    )
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(7 * k)))

    # The bar itself, gold top to bottom like the folder.
    body = gradient(size, GOLD_TOP, GOLD_BOTTOM).convert("RGBA")
    body.putalpha(rounded_mask(size, bar, radius))
    canvas.alpha_composite(body)

    # Cream highlight along the top edge — the folder's light band, borrowed.
    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(glow).rounded_rectangle(
        (bar[0] + 10 * k, bar[1] + 8 * k, bar[2] - 10 * k, bar[1] + 26 * k),
        radius=9 * k,
        fill=TILE + (150,),
    )
    canvas.alpha_composite(Image.composite(glow, Image.new("RGBA", (size, size), (0, 0, 0, 0)), rounded_mask(size, bar, radius)))

    # Three app tiles. They grow slightly towards the middle, like a magnified
    # dock, and stay big enough to read at 16 px.
    tile_w = 46 * k
    gap = 14 * k
    total = tile_w * 3 + gap * 2
    left = (size - total) / 2
    baseline = bar[3] - 24 * k
    for index, lift in enumerate((6 * k, 0, 6 * k)):
        x = left + index * (tile_w + gap)
        height = tile_w + (6 * k if lift else 0)
        box = (x, baseline - height, x + tile_w, baseline)
        tile = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        draw = ImageDraw.Draw(tile)
        draw.rounded_rectangle(box, radius=13 * k, fill=TILE + (255,))
        draw.rounded_rectangle(
            (box[0], box[1], box[2], box[1] + 10 * k), radius=6 * k, fill=TILE_EDGE + (255,)
        )
        canvas.alpha_composite(tile)

    return canvas


def draw_service_icon(units=BOX, scale=SUPERSAMPLE):
    """The service: a gold gear. No text, no letters — it has to read at 16 px."""
    import math

    size = units * scale
    k = size / BOX
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    centre = size / 2
    outer = 106 * k
    inner = 74 * k
    teeth = 9
    points = []
    # Two vertices per tooth at equal angles: a clean cog, not a jagged star.
    for step in range(teeth * 2):
        radius = outer if step % 2 == 0 else inner
        angle = math.radians(step * 180 / teeth - 90)
        points.append((centre + radius * math.cos(angle), centre + radius * math.sin(angle)))

    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon([(x, y + 8 * k) for x, y in points], fill=SHADOW + (110,))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(7 * k)))

    body = gradient(size, GOLD_TOP, GOLD_BOTTOM).convert("RGBA")
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).polygon(points, fill=255)
    body.putalpha(mask)
    canvas.alpha_composite(body)

    # Hub: a cream ring in the middle, the hole the gear turns around.
    hub = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(hub)
    draw.ellipse((centre - 44 * k, centre - 44 * k, centre + 44 * k, centre + 44 * k), fill=TILE + (255,))
    draw.ellipse((centre - 23 * k, centre - 23 * k, centre + 23 * k, centre + 23 * k), fill=(0, 0, 0, 0))
    canvas.alpha_composite(hub)
    return canvas


def main():
    write("crates/aura-dock/resources/aura-dock.ico", draw_icon)
    write("crates/aura-service/resources/auraservice.ico", draw_service_icon)


def write(path, paint):
    import os
    frames = {}
    for size in SIZES:
        frames[size] = paint(size, SUPERSAMPLE).resize((size, size), Image.LANCZOS)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    largest = frames[256]
    largest.save(
        path,
        format="ICO",
        sizes=[(size, size) for size in SIZES],
        append_images=[frames[size] for size in SIZES if size != 256],
    )
    print(f"{path}: {len(SIZES)} sizes")
    # A preview strip, handy when adjusting the drawing.
    strip = Image.new("RGBA", (256 + 64 + 48 + 32 + 16 + 40, 64), (0, 0, 0, 0))
    x = 0
    for size in (256, 64, 48, 32, 16):
        preview = frames[size]
        strip.alpha_composite(preview, (x, 0)) if size <= strip.height else None
        x += preview.width + 10
    strip.save(path.replace(".ico", "-preview.png"))


if __name__ == "__main__":
    main()
