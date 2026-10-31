#!/usr/bin/env python3
"""Extractor for Dark Oberon .dat texture archives -> PNGs + manifest.json.

Format (from src/dodata.cpp of Dark Oberon, GPL):
  magic "Dark Oberon data file" | u8 version | i32 textures_seek
  at seek: i32 group_count; per group: [u8 len][name][i32 tex_count]
  per tex: [u8 len][id][u8 fh][u8 fv][i32 anim_ms][i32 px][i32 py][u8 ttype][u32 dsize][TGA]
"""
import io, json, os, struct, sys
from PIL import Image

MAGIC = b"Dark Oberon data file"

class R:
    def __init__(self, buf): self.b = buf; self.p = 0
    def u8(self): v = self.b[self.p]; self.p += 1; return v
    def i32(self): v = struct.unpack_from("<i", self.b, self.p)[0]; self.p += 4; return v
    def u32(self): v = struct.unpack_from("<I", self.b, self.p)[0]; self.p += 4; return v
    def seek(self, n): self.p = n
    def s(self):
        n = self.u8(); s = self.b[self.p:self.p+n].decode("latin-1"); self.p += n; return s

def decode_tga(blob):
    # TGA header: 18 bytes
    id_len, cmap, img_type = blob[0], blob[1], blob[2]
    cw, ch = struct.unpack_from("<HH", blob, 12)
    bpp = blob[16]; desc = blob[17]
    if cw == 0 or ch == 0: raise ValueError("empty tga")
    top = bool(desc & 0x20)
    p = 18 + id_len
    total = cw * ch
    px = bytearray(total * 4)
    def put(n, r, g, b, a):
        x = n % cw; y = n // cw
        yy = y if top else ch - 1 - y
        o = (yy * cw + x) * 4
        px[o:o+4] = bytes((r, g, b, a))
    if img_type in (2, 3):
        bppc = 1 if img_type == 3 else bpp // 8
        for n in range(total):
            c = blob[p:p+bppc]; p += bppc
            if img_type == 3: put(n, c[0], c[0], c[0], 255)
            else:
                b, g, r = c[0], c[1], c[2]
                a = c[3] if bppc == 4 else 255
                put(n, r, g, b, a)
    elif img_type in (10, 11):
        bppc = 1 if img_type == 11 else bpp // 8
        n = 0
        while n < total:
            packet = blob[p]; p += 1
            cnt = (packet & 0x7F) + 1
            if packet & 0x80:
                c = blob[p:p+bppc]; p += bppc
                vals = (c[0], c[0], c[0], 255) if img_type == 11 else (c[2], c[1], c[0], c[3] if bppc==4 else 255)
                for _ in range(cnt):
                    if n >= total: break
                    put(n, *vals); n += 1
            else:
                for _ in range(cnt):
                    if n >= total: break
                    c = blob[p:p+bppc]; p += bppc
                    if img_type == 11: put(n, c[0], c[0], c[0], 255)
                    else: put(n, c[2], c[1], c[0], c[3] if bppc==4 else 255)
                    n += 1
    elif img_type in (1, 9):
        first, clen, cdepth = struct.unpack_from("<HB", blob, 1) + (blob[3],)
        cmap_entry = max(1, cdepth // 8)
        table_start = 18 + id_len
        def cm(v):
            o = table_start + v * cmap_entry
            c = blob[o:o+cmap_entry]
            if cdepth == 24: return c[2], c[1], c[0], 255
            if cdepth == 32: return c[2], c[1], c[0], c[3]
            return c[0], c[0], c[0], 255
        p = table_start + clen * cmap_entry
        if img_type == 1:
            for n in range(total):
                put(n, *cm(blob[p])); p += 1
        else:
            n = 0
            while n < total:
                packet = blob[p]; p += 1
                cnt = (packet & 0x7F) + 1
                if packet & 0x80:
                    v = cm(blob[p]); p += 1
                    for _ in range(cnt):
                        if n >= total: break
                        put(n, *v); n += 1
                else:
                    for _ in range(cnt):
                        if n >= total: break
                        put(n, *cm(blob[p])); p += 1; 
    else:
        raise ValueError(f"unsupported tga type {img_type}")
    img = Image.frombytes("RGBA", (cw, ch), bytes(px))
    return img

def sanitize(s):
    return "".join(c if c.isalnum() or c in "_-" else "_" for c in s)

def main(inp, outdir):
    data = open(inp, "rb").read()
    assert data[:len(MAGIC)] == MAGIC, "not a Dark Oberon dat file"
    r = R(data); r.seek(len(MAGIC))
    version = r.u8(); print("format version", version)
    seek = r.i32(); r.seek(seek)
    gcount = r.i32()
    manifest = {"source": inp, "groups": []}
    for _ in range(gcount):
        gname = r.s(); tcount = r.i32()
        print(f"group '{gname}' with {tcount} textures")
        gdir = os.path.join(outdir, sanitize(gname))
        os.makedirs(gdir, exist_ok=True)
        gentries = []
        for _ in range(tcount):
            tid = r.s(); fh = r.u8(); fv = r.u8(); anim = r.i32()
            r.i32(); r.i32(); r.u8()
            dsize = r.u32()
            blob = data[r.p:r.p+dsize]; r.p += dsize
            try:
                img = decode_tga(blob)
            except Exception as e:
                print("  skip", tid, e); continue
            fname = sanitize(tid) + ".png"
            img.save(os.path.join(gdir, fname))
            gentries.append({"id": tid, "frames_h": fh, "frames_v": fv,
                             "anim_ms": anim, "file": f"{sanitize(gname)}/{fname}"})
        manifest["groups"].append({"name": gname, "textures": gentries})
    with open(os.path.join(outdir, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2)
    print("done ->", outdir)

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
