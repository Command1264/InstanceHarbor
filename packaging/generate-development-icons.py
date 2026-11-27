"""Derive the red development-channel icons from the blue product icons.

Only the hue changes: every pixel keeps its saturation, value and alpha, and the
ICO keeps every size of the original. Uses the Python standard library only.

    python packaging/generate-development-icons.py            # rewrite the red files
    python packaging/generate-development-icons.py --check    # verify the committed files
    python packaging/generate-development-icons.py --comparison out.png [--product a.ico --development b.ico]
"""
import argparse
import colorsys
import json
import math
from pathlib import Path
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parent.parent
# The channel icon files come from the one manifest every consumer reads (#116).
CHANNEL_ICONS = json.loads((ROOT / "src-tauri/icons/channels.json").read_text(encoding="utf-8"))
# (blue product source, red development output): the ICO first, then the sidebar image.
PAIRS = tuple(
    (ROOT / CHANNEL_ICONS["product"][kind], ROOT / CHANNEL_ICONS["development"][kind])
    for kind in ("app", "brand")
)
# The product blue sits near 220°; turning it by 140° lands on red (0°).
HUE_SHIFT_DEGREES = 140
PRODUCT_HUES = (190, 250)
DEVELOPMENT_HUES = (-20, 20)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


class Image:
    def __init__(self, width, height, pixels):
        self.width, self.height, self.pixels = width, height, pixels  # pixels: bytearray RGBA


def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)


def decode_png(data):
    if not data.startswith(PNG_SIGNATURE):
        raise ValueError("not a PNG image")
    position, compressed, header = 8, b"", None
    while position < len(data):
        length, kind = struct.unpack(">I4s", data[position:position + 8])
        body = data[position + 8:position + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            compressed += body
        position += 12 + length
    width, height, depth, color, _, _, interlace = header
    if (depth, color, interlace) != (8, 6, 0):
        raise ValueError(f"only non-interlaced 8-bit RGBA PNG is supported, got depth {depth}, color {color}")
    raw, stride = zlib.decompress(compressed), width * 4
    pixels, previous, offset = bytearray(), bytearray(stride), 0
    for _ in range(height):
        kind, line = raw[offset], bytearray(raw[offset + 1:offset + 1 + stride])
        offset += 1 + stride
        for x in range(stride):
            left = line[x - 4] if x >= 4 else 0
            up = previous[x]
            corner = previous[x - 4] if x >= 4 else 0
            if kind == 1:
                line[x] = (line[x] + left) & 255
            elif kind == 2:
                line[x] = (line[x] + up) & 255
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 255
            elif kind == 4:
                line[x] = (line[x] + _paeth(left, up, corner)) & 255
            elif kind != 0:
                raise ValueError(f"unknown PNG filter {kind}")
        pixels += line
        previous = line
    return Image(width, height, pixels)


def _chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)


def encode_png(image):
    stride = image.width * 4
    rows = b"".join(b"\0" + bytes(image.pixels[y * stride:(y + 1) * stride]) for y in range(image.height))
    header = struct.pack(">IIBBBBB", image.width, image.height, 8, 6, 0, 0, 0)
    return PNG_SIGNATURE + _chunk(b"IHDR", header) + _chunk(b"IDAT", zlib.compress(rows, 9)) + _chunk(b"IEND", b"")


def read_ico(data):
    """The directory entries (raw 16 bytes) and PNG images of an ICO file."""
    reserved, kind, count = struct.unpack("<HHH", data[:6])
    if (reserved, kind) != (0, 1):
        raise ValueError("not an ICO file")
    entries = []
    for index in range(count):
        entry = data[6 + 16 * index:22 + 16 * index]
        size, offset = struct.unpack("<II", entry[8:16])
        image = data[offset:offset + size]
        if not image.startswith(PNG_SIGNATURE):
            raise ValueError(f"ICO image {index} is not PNG; only PNG entries are supported")
        entries.append((entry, image))
    return entries


def write_ico(entries):
    """Rebuilds an ICO with the original directory fields and new image data."""
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset, directory, images = 6 + 16 * len(entries), b"", b""
    for entry, image in entries:
        directory += entry[:8] + struct.pack("<II", len(image), offset)
        images += image
        offset += len(image)
    return header + directory + images


def shift_hue(image, degrees):
    shifted, turn = bytearray(image.pixels), degrees / 360
    for index in range(0, len(shifted), 4):
        red, green, blue = (value / 255 for value in shifted[index:index + 3])
        hue, saturation, value = colorsys.rgb_to_hsv(red, green, blue)
        red, green, blue = colorsys.hsv_to_rgb((hue + turn) % 1.0, saturation, value)
        shifted[index:index + 3] = bytes(round(channel * 255) for channel in (red, green, blue))
    return Image(image.width, image.height, shifted)


def images_of(path):
    data = path.read_bytes()
    if path.suffix == ".ico":
        return [decode_png(image) for _, image in read_ico(data)]
    return [decode_png(data)]


def derive(path):
    """The red file bytes derived from a blue source file."""
    data = path.read_bytes()
    if path.suffix == ".ico":
        return write_ico([(entry, encode_png(shift_hue(decode_png(image), HUE_SHIFT_DEGREES)))
                          for entry, image in read_ico(data)])
    return encode_png(shift_hue(decode_png(data), HUE_SHIFT_DEGREES))


def dominant_hue(image):
    """The alpha-weighted mean hue in degrees (-180..180) of the coloured pixels."""
    x = y = 0.0
    for index in range(0, len(image.pixels), 4):
        red, green, blue, alpha = image.pixels[index:index + 4]
        hue, saturation, _ = colorsys.rgb_to_hsv(red / 255, green / 255, blue / 255)
        weight = alpha * saturation
        x += weight * math.cos(hue * 2 * math.pi)
        y += weight * math.sin(hue * 2 * math.pi)
    return math.degrees(math.atan2(y, x))


def _in_range(hue, low, high):
    hue = (hue + 180) % 360 - 180
    low_hue = (low + 180) % 360 - 180
    high_hue = (high + 180) % 360 - 180
    return low_hue <= hue <= high_hue if low_hue <= high_hue else hue >= low_hue or hue <= high_hue


def check():
    problems = []
    for source, target in PAIRS:
        if not target.exists():
            problems.append(f"{target.relative_to(ROOT)} is missing; run this script without --check.")
            continue
        blue, red = images_of(source), images_of(target)
        expected = [shift_hue(image, HUE_SHIFT_DEGREES) for image in blue]
        sizes = [(image.width, image.height) for image in blue]
        if sizes != [(image.width, image.height) for image in red]:
            problems.append(f"{target.relative_to(ROOT)} does not hold the sizes {sizes} of {source.relative_to(ROOT)}.")
            continue
        if any(a.pixels != b.pixels for a, b in zip(expected, red)):
            problems.append(f"{target.relative_to(ROOT)} is out of date with {source.relative_to(ROOT)}; run this script.")
        if source.suffix == ".ico" and [e[:8] for e, _ in read_ico(source.read_bytes())] != [e[:8] for e, _ in read_ico(target.read_bytes())]:
            problems.append(f"{target.relative_to(ROOT)} directory fields differ from {source.relative_to(ROOT)}.")
        for image in blue:
            if not _in_range(dominant_hue(image), *PRODUCT_HUES):
                problems.append(f"{source.relative_to(ROOT)} {image.width}px is no longer blue (hue {dominant_hue(image):.0f}°).")
        for image in red:
            if not _in_range(dominant_hue(image), *DEVELOPMENT_HUES):
                problems.append(f"{target.relative_to(ROOT)} {image.width}px is not red (hue {dominant_hue(image):.0f}°).")
    for problem in problems:
        print(problem, file=sys.stderr)
    if not problems:
        print("Development icons match the product icons with only the hue changed.")
    return 1 if problems else 0


def comparison(output, product, development):
    """A PNG with the product icon sizes on top and the development ones below,
    on a light and a dark background."""
    rows = [images_of(product), images_of(development)]
    gap, columns = 16, max(len(row) for row in rows)
    cell = max(image.width for row in rows for image in row)
    width = gap + sum(max(row[i].width for row in rows) + gap for i in range(columns))
    band = gap + 2 * (cell + gap)
    height = 2 * band
    canvas = bytearray()
    for y in range(height):
        shade = 243 if y < band else 32
        canvas += bytes((shade, shade, shade, 255)) * width
    for panel in range(2):
        for row_index, row in enumerate(rows):
            top = panel * band + gap + row_index * (cell + gap)
            left = gap
            for column, image in enumerate(row):
                y_offset = top + cell - image.height
                for y in range(image.height):
                    for x in range(image.width):
                        source = (y * image.width + x) * 4
                        target = ((y_offset + y) * width + left + x) * 4
                        alpha = image.pixels[source + 3] / 255
                        for channel in range(3):
                            canvas[target + channel] = round(image.pixels[source + channel] * alpha
                                                             + canvas[target + channel] * (1 - alpha))
                left += max(r[column].width for r in rows) + gap
    output.write_bytes(encode_png(Image(width, height, canvas)))
    print(f"Comparison written to {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="verify the committed red files instead of writing them")
    parser.add_argument("--comparison", type=Path, help="write a blue/red side-by-side PNG to this path")
    parser.add_argument("--product", type=Path, default=PAIRS[0][0], help="blue ICO or PNG for --comparison")
    parser.add_argument("--development", type=Path, default=PAIRS[0][1], help="red ICO or PNG for --comparison")
    arguments = parser.parse_args()
    if arguments.comparison:
        comparison(arguments.comparison, arguments.product, arguments.development)
        return 0
    if arguments.check:
        return check()
    for source, target in PAIRS:
        target.write_bytes(derive(source))
        print(f"Wrote {target.relative_to(ROOT)}")
    return check()


if __name__ == "__main__":
    sys.exit(main())
