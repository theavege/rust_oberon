#!/usr/bin/env python3
"""Extract sound entries from Dark Oberon .dat files (v3: second i32 = sounds_seek).
Entry: [u8 len][id][i8 format][i8 stype][i32 dsize][data]
stype: -1 module, 0 sample, 1 stream. Extension by magic sniff."""
import os, struct, sys

MAGIC = b"Dark Oberon data file"

def main(inp, outdir):
    data = open(inp, "rb").read()
    assert data[:len(MAGIC)] == MAGIC
    p = len(MAGIC)
    version = data[p]; p += 1
    tex_seek = struct.unpack_from("<i", data, p)[0]; p += 4
    snd_seek = struct.unpack_from("<i", data, p)[0] if version > 2 else 0
    print("version", version, "sounds_seek", snd_seek)
    if not snd_seek:
        print("no sounds"); return
    p = snd_seek
    count = struct.unpack_from("<i", data, p)[0]; p += 4
    os.makedirs(outdir, exist_ok=True)
    manifest = []
    for _ in range(count):
        n = data[p]; p += 1
        sid = data[p:p+n].decode("latin-1"); p += n
        sformat = struct.unpack_from("<b", data, p)[0]; p += 1
        stype = struct.unpack_from("<b", data, p)[0]; p += 1
        dsize = struct.unpack_from("<i", data, p)[0]; p += 4
        blob = data[p:p+dsize]; p += dsize
        if stype == 1:
            # stream: data is skipped in C loader too (loaded lazily via seek);
            # but the bytes ARE present in file, so we can still dump them
            pass
        ext = ".ogg" if blob[:4] == b"OggS" else ".wav" if blob[:4] == b"RIFF" else ".mod" if stype == -1 else ".bin"
        safe = "".join(c if c.isalnum() or c in "_-" else "_" for c in sid) + ext
        with open(os.path.join(outdir, safe), "wb") as f:
            f.write(blob)
        manifest.append({"id": sid, "type": stype, "format": sformat, "file": safe})
    import json
    json.dump({"source": inp, "sounds": manifest}, open(os.path.join(outdir, "manifest.json"), "w"), indent=2)
    print("extracted", len(manifest), "sounds ->", outdir)

main(sys.argv[1], sys.argv[2])
