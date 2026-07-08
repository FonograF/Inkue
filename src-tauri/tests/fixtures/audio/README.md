# Real audio fixtures (compressed formats)

The WAV-based decode tests generate their own fixtures and need nothing here.
To also exercise the **real symphonia code paths for compressed formats**, drop a
few short (1–3 s is plenty) real audio files into this folder:

| Extension | Codec exercised            |
|-----------|----------------------------|
| `.flac`   | FLAC                       |
| `.mp3`    | MP3 (MPEG-1 Layer III)     |
| `.ogg`    | Vorbis in Ogg              |
| `.m4a`    | AAC in MP4/ISO-BMFF        |

The filename does not matter — the tests pick the first file with each
extension. Any of these missing → its test is **skipped** (printed to stderr),
never failed, so the suite stays green without them.

These files are intentionally **not committed** (see `.gitignore`); they are
your local test material.
