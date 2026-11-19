#!/usr/bin/env python3
"""Draw the panel (dock) icon and write `crates/zsdesk-dock/resources/zsdesk-dock.ico`.

The file manager's own icon (`crates/zsdesk-app/resources/zsdesk.ico`) is a golden
macOS-style folder; the panel's icon is the same gold, drawn as the panel itself —
a rounded bar with three app tiles on it — so the two read as one product in Explorer
and in the Start menu.

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


def main():
    frames = {}
    for size in SIZES:
        frames[size] = draw_icon(size, SUPERSAMPLE).resize(
            (size, size), Image.LANCZOS
        )
    largest = frames[256]
    largest.save(
        "crates/zsdesk-dock/resources/zsdesk-dock.ico",
        format="ICO",
        sizes=[(size, size) for size in SIZES],
        append_images=[frames[size] for size in SIZES if size != 256],
    )
    # A preview strip, handy when adjusting the drawing.
    strip = Image.new("RGBA", (256 + 64 + 48 + 32 + 16 + 40, 64), (0, 0, 0, 0))
    x = 0
    for size in (256, 64, 48, 32, 16):
        preview = frames[size]
        strip.alpha_composite(preview, (x, 0)) if size <= strip.height else None
        x += preview.width + 10
    print("wrote crates/zsdesk-dock/resources/zsdesk-dock.ico")


if __name__ == "__main__":
    main()
