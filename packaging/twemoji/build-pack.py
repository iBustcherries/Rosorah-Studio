#!/usr/bin/env python3
"""Packs Twemoji's 72x72 PNGs and Unicode's emoji-test.txt into crates/app/assets/twemoji/twemoji.pack.

Usage: build-pack.py <twemoji checkout> <emoji-test.txt> <output>
Format (little endian): b"RTW1"; u8 group count, then u8 length + name per group;
u32 image count, then per image u8 key length + key (UTF-8, U+FE0F removed) + u8 flags + u32 length + PNG
(flag 1: a single character that is only an emoji when U+FE0F follows it, such as © or ☺);
u32 picker count, then per entry u8 group + u8 length + emoji (fully qualified) + u8 length + name.
"""
import os, struct, sys

def key(chars):
    return ''.join(c for c in chars if c != '️')

def main(root, test, out):
    folder = os.path.join(root, 'assets', '72x72')
    images = {}
    for name in sorted(os.listdir(folder)):
        if name.endswith('.png'):
            chars = ''.join(chr(int(h, 16)) for h in name[:-4].split('-'))
            images[key(chars)] = open(os.path.join(folder, name), 'rb').read()
    groups, picker, group, text_default = [], [], None, set()
    for line in open(test, encoding='utf-8'):
        if '; fully-qualified' in line:
            codes = line.split(';', 1)[0].split()
            if len(codes) == 2 and codes[1] == 'FE0F':
                text_default.add(chr(int(codes[0], 16)))
        if line.startswith('# group:'):
            group = line.split(':', 1)[1].strip()
            if group != 'Component':
                groups.append(group)
            continue
        if group in (None, 'Component') or line.startswith('#') or '; fully-qualified' not in line:
            continue
        codes, comment = line.split(';', 1)[0], line.split('#', 1)[1].strip()
        chars = ''.join(chr(int(h, 16)) for h in codes.split())
        # Skin tones are left out of the picker; chat still shows them.
        if any(0x1F3FB <= ord(c) <= 0x1F3FF for c in chars) or key(chars) not in images:
            continue
        name = comment.split(' ', 2)[2]
        picker.append((groups.index(group), chars, name))
    data = bytearray(b'RTW1')
    data += struct.pack('<B', len(groups))
    for g in groups:
        g = g.encode(); data += struct.pack('<B', len(g)) + g
    data += struct.pack('<I', len(images))
    for k, png in images.items():
        flags = 1 if k in text_default else 0
        k = k.encode(); data += struct.pack('<B', len(k)) + k + struct.pack('<BI', flags, len(png)) + png
    data += struct.pack('<I', len(picker))
    for g, chars, name in picker:
        chars, name = chars.encode(), name.encode()
        data += struct.pack('<BB', g, len(chars)) + chars + struct.pack('<B', len(name)) + name
    open(out, 'wb').write(data)
    print(f'{len(images)} images, {len(picker)} picker entries, {len(data)} bytes')

if __name__ == '__main__':
    main(*sys.argv[1:4])
