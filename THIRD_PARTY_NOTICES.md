# Third-Party Notices

velio. bundles or downloads the third-party components listed below. This file is installed with the
application (`THIRD_PARTY_NOTICES.md` in the install folder). It covers the bundled programs (FFmpeg, whisper.cpp,
libmpv) and the libraries built into them, the main libraries compiled into the application, and the AI models the
application can download.

The complete per-package copyright notices and license texts for all Rust crates and npm packages compiled into the
application (the Rust crates linked into the application and the npm packages bundled into its user interface,
almost all under MIT, Apache-2.0, BSD, ISC, Zlib or Unicode-3.0; the exact counts are in that file's header) are in
`THIRD_PARTY_LICENSES.md` in the same folder, which is also shown in the application under Settings > Open-source
licenses. That file also contains the full text of every license named in section 1 of this file (including the
GNU GPL v2.0 and v3.0 and the GNU LGPL v2.1 and v3.0), and the ONNX Runtime third-party notices
(`ThirdPartyNotices.txt`).

This is a license notice, not legal advice. Codec patents (H.264, HEVC, AAC, ...) are separate from copyright
licenses and are not covered here.

## 1. Bundled programs and libraries (in the install folder; 1.1-1.2 run as separate processes, 1.3 is loaded dynamically by a separate worker process)

### 1.1 FFmpeg / FFprobe - GPL-3.0-or-later (LGPL-3.0-or-later FFmpeg combined with GPL-2.0-or-later FFTW 3)

- Programs: `ffmpeg.exe`, `ffprobe.exe` and the shared libraries `avcodec-62.dll`, `avdevice-62.dll`,
  `avfilter-11.dll`, `avformat-62.dll`, `avutil-60.dll`, `swresample-6.dll`, `swscale-9.dll` in the install folder.
- Version: FFmpeg n8.1.3 (git `29e619e767`), the official "win64-lgpl-shared" build of
  [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), release `autobuild-2026-09-30-13-08`.
- License: **GNU General Public License v3 or later** for these programs and DLLs as distributed. FFmpeg itself is
  licensed under the GNU Lesser General Public License v3 or later (the build is configured with `--enable-version3`
  and without `--enable-gpl` / `--enable-nonfree`), but this build also enables Chromaprint, which BtbN builds with
  the FFTW 3 library (GNU GPL v2 or later); FFTW code is statically contained in `avformat-62.dll`. The combined
  programs and DLLs are therefore distributed under the terms of the GNU GPL v3 or later (see the notes on 1.1.1).
  Full texts in the install folder: `licenses\GPL-3.0.txt`, `licenses\GPL-2.0.txt`, `licenses\ffmpeg-LICENSE.txt`
  (FFmpeg's own license file). FFmpeg is copyright (c) the FFmpeg developers; FFTW is copyright (c) Matteo Frigo and
  the Massachusetts Institute of Technology.
- velio. does not link FFmpeg: it starts `ffmpeg.exe` / `ffprobe.exe` as separate programs and exchanges data with
  them only through command-line arguments, files and pipes, to read metadata and decode frames. The rest of velio.
  is not a derivative of these programs and is not distributed under the GPL.
- **Source code**: the exact source of this build is the FFmpeg git commit `29e619e767` (tag `n8.1.3` + 9 commits),
  <https://github.com/FFmpeg/FFmpeg/commit/29e619e767>, and the build scripts / patches / dependency versions are in
  <https://github.com/BtbN/FFmpeg-Builds> (tag `autobuild-2026-09-30-13-08`). You can also request the complete
  corresponding source code of FFmpeg and of the LGPL / GPL licensed libraries listed in 1.1.1 from the velio.
  distributor by opening an issue at <https://github.com/laypauls/velio-mam-desktop/issues> (written offer valid for at least three years after the release).
- **Replacing the libraries**: the libraries are shared DLLs. You may replace them (or `ffmpeg.exe` / `ffprobe.exe`)
  with your own build of a compatible FFmpeg version; the application also accepts an explicit FFmpeg path in its
  settings.
- Encoders and filters that require the GPL (x264, x265, xvid, xavs2, davs2, vid.stab, rubberband, frei0r,
  AviSynth, libdvdread / libdvdnav) are **not** present (`--disable-*` in the build configuration).

#### 1.1.1 Libraries built into the FFmpeg DLLs

The FFmpeg DLLs statically contain the libraries below. The list was taken from the build configuration embedded in
the shipped binaries (`ffmpeg -buildconf`) and the dependency scripts of the BtbN repository at the release tag
(`scripts.d/*.sh`, which pin each library to a git commit or tag). "Version" is that commit (first 12 characters) or
tag; the source is the listed repository at that version.

| Library | Version | License (SPDX) | Copyright | Source |
|---|---|---|---|---|
| zlib | `767c4c947852` | Zlib | Jean-loup Gailly and Mark Adler | <https://github.com/madler/zlib> |
| GNU libiconv (library) | `1df3087ba811` | LGPL-2.1-or-later | Free Software Foundation, Inc. | <https://git.savannah.gnu.org/git/libiconv.git> |
| libxml2 | `91586dc6742a` | MIT | Daniel Veillard; The Libxml2 Contributors | <https://github.com/GNOME/libxml2> |
| liblzma (XZ Utils) | `3b1efb04d17c` | 0BSD | The XZ Utils authors and contributors | <https://github.com/tukaani-project/xz> |
| libvmaf | `86da14d0306a` | BSD-2-Clause-Patent | Netflix, Inc. | <https://github.com/Netflix/vmaf> |
| Fontconfig | `bd8f7b597de9` | MIT (Fontconfig license, see `COPYING`) | Keith Packard; Red Hat, Inc.; Google, Inc. and others | <https://gitlab.freedesktop.org/fontconfig/fontconfig> |
| FreeType | `d33343963303` | FTL (chosen from FTL OR GPL-2.0-or-later) | The FreeType Project (David Turner, Robert Wilhelm, Werner Lemberg) | <https://gitlab.freedesktop.org/freetype/freetype> |
| HarfBuzz | `b3bab6230701` | MIT-Modern-Variant ("Old MIT") | Google, Inc.; Red Hat, Inc.; Behdad Esfahbod and others | <https://github.com/harfbuzz/harfbuzz> |
| GNU FriBidi | `4c914a92e94a` | LGPL-2.1-or-later | The FriBidi authors | <https://github.com/fribidi/fribidi> |
| Brotli | `a3abcbee0d94` | MIT | The Brotli Authors | <https://github.com/google/brotli> |
| libpng | `964b41359497` | libpng-2.0 | The PNG Reference Library Authors; Cosmin Truta; Glenn Randers-Pehrson and others | <https://github.com/glennrp/libpng> |
| Vulkan-Headers | `v1.4.363` | Apache-2.0 OR MIT | The Khronos Group Inc. | <https://github.com/KhronosGroup/Vulkan-Headers> |
| Vulkan-Shim-Loader | `65b3936528cd` | MIT | The Vulkan-Shim-Loader authors | <https://github.com/BtbN/Vulkan-Shim-Loader> |
| shaderc | `a8abeb0b8a9d` | Apache-2.0 | The Shaderc Authors (Google LLC) | <https://github.com/google/shaderc> |
| glslang (via shaderc) | `e1b562a8bed2` | BSD-3-Clause AND BSD-2-Clause AND MIT AND Apache-2.0 AND GPL-3.0-or-later WITH Bison-exception-2.2 (per file, see its `LICENSE.txt`) | The Khronos Group Inc.; 3Dlabs Inc. Ltd.; LunarG, Inc.; Google, Inc. and others | <https://github.com/KhronosGroup/glslang> |
| SPIRV-Tools (via shaderc) | `ef96ed763b43` | Apache-2.0 | The Khronos Group Inc.; Google LLC and others | <https://github.com/KhronosGroup/SPIRV-Tools> |
| SPIRV-Headers | `2b7475f6d664` | MIT (Khronos variant, see `LICENSE`) | The Khronos Group Inc. | <https://github.com/KhronosGroup/SPIRV-Headers> |
| SPIRV-Cross | `aa217aeb6c9f` | Apache-2.0 | Arm Limited and others | <https://github.com/KhronosGroup/SPIRV-Cross> |
| dav1d | `9711965b60bb` | BSD-2-Clause | VideoLAN and dav1d authors | <https://code.videolan.org/videolan/dav1d> |
| libvorbis | `1b75110b5a27` | BSD-3-Clause | Xiph.Org Foundation | <https://github.com/xiph/vorbis> |
| libogg | `06a5e0262cdc` | BSD-3-Clause | Xiph.Org Foundation | <https://github.com/xiph/ogg> |
| rav1e | `31435de9d76f` | BSD-2-Clause (+ Alliance for Open Media Patent License 1.0) | The rav1e contributors | <https://github.com/xiph/rav1e> |
| librsvg | `7612431eb02d` | LGPL-2.1-or-later | The librsvg authors | <https://github.com/GNOME/librsvg> |
| GLib (+ proxy-libintl) | `5a124d1e66a6` | LGPL-2.1-or-later | The GLib authors | <https://github.com/GNOME/glib> |
| cairo | `74755964edef` | LGPL-2.1-only (chosen from LGPL-2.1-only OR MPL-1.1) | The cairo authors | <https://gitlab.freedesktop.org/cairo/cairo> |
| Pango | `8e74c27c113c` | LGPL-2.0-or-later | The Pango authors | <https://github.com/GNOME/pango> |
| pixman | `96c04d1b8793` | MIT | Keith Packard; The Open Group; Digital Equipment Corporation and others | <https://gitlab.freedesktop.org/pixman/pixman> |
| libffi | `bc5538673672` | MIT | Anthony Green, Red Hat, Inc and others | <https://github.com/libffi/libffi> |
| PCRE2 | `09eb19dc1102` | BSD-3-Clause WITH PCRE2-exception | University of Cambridge (Philip Hazel) and contributors | <https://github.com/PCRE2Project/pcre2> |
| GMP | `763b40df71c0` | LGPL-3.0-or-later OR GPL-2.0-or-later (used under LGPL-3.0-or-later) | Free Software Foundation, Inc. | <https://github.com/BtbN/gmplib> |
| LCEVCdec | `17804ac54db8` | BSD-3-Clause-Clear | V-Nova International Limited | <https://github.com/v-novaltd/LCEVCdec> |
| OpenCL-Headers / OpenCL-ICD-Loader | `e6060189f4eb` / `5192c84f8059` | Apache-2.0 | The Khronos Group Inc. | <https://github.com/KhronosGroup/OpenCL-Headers>, <https://github.com/KhronosGroup/OpenCL-ICD-Loader> |
| AMF (headers) | `6277e353fd62` | MIT | Advanced Micro Devices, Inc. | <https://github.com/GPUOpen-LibrariesAndSDKs/AMF> |
| nv-codec-headers | `eddcea9e27f6` | MIT | NVIDIA Corporation | <https://github.com/FFmpeg/nv-codec-headers> |
| libaom | `b4ce2cd097b0` | BSD-2-Clause (+ Alliance for Open Media Patent License 1.0) | Alliance for Open Media | <https://aomedia.googlesource.com/aom> |
| Chromaprint | `aed8eba2202d` | MIT AND LGPL-2.1-or-later (includes FFmpeg code) | Lukas Lalinsky | <https://github.com/acoustid/chromaprint> |
| FFTW 3 (used by Chromaprint) | `93ed4c786934` | **GPL-2.0-or-later** | Matteo Frigo and Massachusetts Institute of Technology | <https://github.com/FFTW/fftw3> |
| Game_Music_Emu (Nuked YM2612 emulator) | `f68963b1de06` | LGPL-2.1-or-later | Shay Green and contributors | <https://github.com/libgme/game-music-emu> |
| Kvazaar | `2b06691bb584` | BSD-3-Clause | Tampere University and the Kvazaar contributors | <https://github.com/ultravideo/kvazaar> |
| aribb24 | `5e9be272f96e` | LGPL-3.0 (repository `COPYING`); source files state LGPL-2.1-or-later | Naohiro KORIYAMA and others | <https://github.com/nkoriyama/aribb24> |
| libaribcaption | `c64c23b8905b` | MIT | The libaribcaption authors (xqq) | <https://github.com/xqq/libaribcaption> |
| libass | `f61db567e659` | ISC | The libass contributors | <https://github.com/libass/libass> |
| libunibreak | `28a2756b864c` | Zlib | Wu Yongwei and contributors | <https://github.com/adah1972/libunibreak> |
| libbluray | `a24f4fad4d62` | LGPL-2.1-or-later | The libbluray authors | <https://code.videolan.org/videolan/libbluray> |
| libudfread | `b0bc6957e7e0` | LGPL-2.1-or-later | The libudfread authors | <https://code.videolan.org/videolan/libudfread> |
| libjxl | `b87738951c12` | BSD-3-Clause | The JPEG XL Project Authors | <https://github.com/libjxl/libjxl> |
| Highway (libjxl submodule) | submodule of libjxl at the commit above | Apache-2.0 OR BSD-3-Clause | Google LLC | <https://github.com/google/highway> |
| Little CMS | `a0b0d7a69b13` | MIT | Marti Maria Saguer | <https://github.com/mm2/Little-CMS> |
| LAME | SVN r6835 | LGPL-2.0-or-later | The LAME project developers | <https://svn.code.sf.net/p/lame/svn/trunk/lame> |
| Opus | `503d81b138d7` | BSD-3-Clause | Xiph.Org; Skype Limited; Octasic; Jean-Marc Valin; Timothy B. Terriberry and others | <https://github.com/xiph/opus> |
| libplacebo (+ bundled fast_float, glad loader) | `e2972fdd09ad` | LGPL-2.1-or-later (fast_float: Apache-2.0 OR BSL-1.0 OR MIT) | Niklas Haas and contributors | <https://code.videolan.org/videolan/libplacebo> |
| librist | `4f45ef8f7898` | BSD-2-Clause | VideoLAN and librist authors | <https://code.videolan.org/rist/librist> |
| Mbed TLS | `v4.2.0` | Apache-2.0 OR GPL-2.0-or-later (used under Apache-2.0) | The Mbed TLS Contributors | <https://github.com/Mbed-TLS/mbedtls> |
| libssh | `76e0a8639e61` | LGPL-2.1-or-later | Aris Adamantiadis, Andreas Schneider and others | <https://gitlab.com/libssh/libssh-mirror> |
| OpenSSL (used by libssh, SRT, libaribcaption) | `openssl-3.6.4` | Apache-2.0 | The OpenSSL Project Authors | <https://github.com/openssl/openssl> |
| libtheora | `28fd5ec77f0a` | BSD-3-Clause | Xiph.Org Foundation | <https://github.com/xiph/theora> |
| libvpx | `5e680f30801d` | BSD-3-Clause (+ WebM patent grant, `PATENTS`) | The WebM Project authors | <https://chromium.googlesource.com/webm/libvpx> |
| libwebp | `5c168cd23b18` | BSD-3-Clause (+ WebM patent grant, `PATENTS`) | Google Inc. | <https://chromium.googlesource.com/webm/libwebp> |
| ZeroMQ (libzmq) | `46493370217a` | MPL-2.0 | The ZeroMQ authors | <https://github.com/zeromq/libzmq> |
| LV2 / lilv / serd / sord / sratom / zix | `3c57dae600a5` / `4b8f30055fa5` / `b6053bb2a453` / `4b5232bb91e7` / `9e8ee84eed50` / `17d632393f5a` | ISC | David Robillard and other LV2 contributors | <https://github.com/lv2/lv2>, <https://github.com/lv2/lilv>, <https://github.com/drobilla/serd>, <https://github.com/drobilla/sord>, <https://github.com/lv2/sratom>, <https://github.com/drobilla/zix> |
| Intel VPL (libvpl) | `674d015bcb29` | MIT | Intel Corporation | <https://github.com/intel/libvpl> |
| OpenAL Soft | `c89b8cf7bba4` | LGPL-2.0-or-later (default HRTF data: Apache-2.0) | Chris Robinson and contributors | <https://github.com/kcat/openal-soft> |
| OpenAPV | `a58ce739be0d` | BSD-3-Clause | The OpenAPV authors | <https://github.com/AcademySoftwareFoundation/openapv> |
| opencore-amr | `7dba8c322384` | Apache-2.0 | PacketVideo / The Android Open Source Project and contributors | <https://sourceforge.net/projects/opencore-amr/> |
| OpenH264 | `8b2d28faade1` | BSD-2-Clause | Cisco Systems, Inc. | <https://github.com/cisco/openh264> |
| OpenJPEG | `8314119b067c` | BSD-2-Clause | Universite catholique de Louvain (UCL) and other contributors | <https://github.com/uclouvain/openjpeg> |
| libopenmpt | `f83cedb0cd54` | BSD-3-Clause | OpenMPT Project Developers and Contributors | <https://github.com/OpenMPT/openmpt> |
| SDL2 | `b90ac95029d8` (SDL2 branch) | Zlib | Sam Lantinga | <https://github.com/libsdl-org/SDL> |
| Snappy | `9c28114a3886` | BSD-3-Clause | Google Inc. | <https://github.com/google/snappy> |
| SoX Resampler (libsoxr) | `945b592b7047` | LGPL-2.1-or-later | Rob Sykes | <https://sourceforge.net/projects/soxr/> |
| SRT | `ff8ab25c57ae` | MPL-2.0 | Haivision Systems Inc. | <https://github.com/Haivision/srt> |
| SVT-AV1 | `5ed9ffbcb508` | BSD-3-Clause-Clear (+ Alliance for Open Media Patent License 1.0) | Alliance for Open Media | <https://gitlab.com/AOMediaCodec/SVT-AV1> |
| TwoLAME | `6fced852d4d5` | LGPL-2.1-or-later | Michael Cheng; The TwoLAME Project | <https://github.com/njh/twolame> |
| uavs3d | `0e20d2c29185` | BSD-3-Clause | Peking University Shenzhen Graduate School; Peng Cheng Laboratory; Guangdong Bohua UHD Innovation Corporation | <https://github.com/uavs3/uavs3d> |
| libva | `6b07f7100512` | MIT | Intel Corporation | <https://github.com/intel/libva> |
| VVenC | `b1cfb2ad495a` | BSD-3-Clause-Clear | Fraunhofer-Gesellschaft (Fraunhofer HHI) | <https://github.com/fraunhoferhhi/vvenc> |
| zimg (+ graphengine) | `67e0603271c0` | WTFPL | The zimg authors | <https://github.com/sekrit-twc/zimg> |
| libzvbi | `d3a5ee9f2b04` | LGPL-2.0-or-later; `src/packet-830.c` and `src/pdc.c`: **GPL-2.0-only** (see its `COPYING.md`) | Michael H. Schimek and other zvbi authors | <https://github.com/zapping-vbi/zvbi> |
| Zstandard (if linked; see below) | `01b7154f1172` | BSD-3-Clause (chosen from BSD-3-Clause OR GPL-2.0-only) | Meta Platforms, Inc. and affiliates | <https://github.com/facebook/zstd> |
| mingw-w64 runtime and winpthreads | `57b595039040` | ZPL-2.1, MIT and public domain (per component, see its `COPYING` files) | The mingw-w64 project | <https://sourceforge.net/projects/mingw-w64/> |
| mingw-std-threads | `c931bac289dd` | BSD-2-Clause | The mingw-std-threads authors | <https://github.com/meganz/mingw-std-threads> |
| GCC runtime libraries (libgcc, libstdc++, libgomp) | toolchain of the BtbN build image (version not pinned in the build scripts) | GPL-3.0-or-later WITH GCC-exception-3.1 | Free Software Foundation, Inc. | <https://gcc.gnu.org/> |

Notes on 1.1.1:

- Portions of this software are copyright (c) The FreeType Project (<https://freetype.org>). All rights reserved.
- **GPL components.** BtbN builds Chromaprint with FFTW 3, which is licensed under the GNU GPL v2 or later; FFTW code
  is present in `avformat-62.dll` (Chromaprint muxer). Because of this, the FFmpeg programs and DLLs are distributed
  under the GNU GPL v3 or later (section 1.1), and the written offer in 1.1 covers the complete corresponding source
  code of FFTW 3 and of the build scripts. libzvbi (teletext decoder in `avcodec-62.dll`) includes two source files
  licensed GPL-2.0-only; whether their object code is linked into `avcodec-62.dll` was not verified.
- rav1e and librsvg are written in Rust and statically include the Rust crates vendored at the commits above; their
  crate lists and licenses are in the `Cargo.lock` / crate sources of those projects and were not audited one by one.
- Zstandard and Brotli are built by the scripts; whether all of their code is linked into the shipped DLLs was not
  verified (only a few related strings are present in `avformat-62.dll`).
- Not present in this build (not enabled for the LGPL variant or for FFmpeg 8.1, confirmed by the build
  configuration): libcurl and its HTTP/3 dependencies, libsamplerate, Rubber Band, frei0r, vid.stab, AviSynth,
  libdvdcss / libdvdread / libdvdnav, davs2, xavs2, x264, x265, Xvid, fdk-aac, whisper.

### 1.2 whisper.cpp - MIT

- Programs: `whisper-cli.exe` and the DLLs listed below, from the official release `b5454` (whisper.cpp v1.9.5),
  `whisper-bin-x64.zip`, <https://github.com/ggml-org/whisper.cpp/releases/tag/b5454>, in the install folder.
- License: MIT. Copyright (c) 2023-2026 The ggml authors.

| File | Component | License (SPDX) | Copyright | Source |
|---|---|---|---|---|
| `whisper-cli.exe`, `whisper.dll` | whisper.cpp (`src/whisper.cpp`, `examples/cli`) | MIT | The ggml authors | <https://github.com/ggml-org/whisper.cpp/tree/b5454> |
| `ggml.dll`, `ggml-base.dll`, `ggml-cpu-*.dll` | ggml (in the whisper.cpp tree), including llamafile `sgemm` | MIT | The ggml authors; `sgemm`: Copyright 2024 Mozilla Foundation | <https://github.com/ggml-org/whisper.cpp/tree/b5454/ggml> |
| `llama.dll` | copy of llama.cpp used by the `talk-llama` example | MIT | The ggml authors | <https://github.com/ggml-org/whisper.cpp/tree/b5454/examples/talk-llama> |
| `parakeet.dll` | Parakeet speech recognition support (`src/parakeet.cpp`) | MIT | The ggml authors | <https://github.com/ggml-org/whisper.cpp/tree/b5454/src> |
| `whisper-cli.exe` (statically included) | miniaudio | Unlicense OR MIT-0 | David Reid | <https://github.com/ggml-org/whisper.cpp/blob/b5454/examples/miniaudio.h> |
| `whisper-cli.exe` (statically included) | stb_vorbis v1.22 | MIT OR Unlicense | Sean Barrett | <https://github.com/ggml-org/whisper.cpp/blob/b5454/examples/stb_vorbis.c> |
| `SDL2.dll` | SDL 2.28.5 (prebuilt DLL shipped in the same archive) | Zlib | Copyright (C) 1997-2023 Sam Lantinga | <https://github.com/libsdl-org/SDL/releases/tag/release-2.28.5> |

```
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit
persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the
Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### 1.3 libmpv - LGPL-3.0-or-later (as distributed)

- Library: `libmpv\libmpv-2.dll` (and its license text `libmpv\LICENSE.LGPL`) in the install folder. It is used by the
  built-in player to play video formats the web view cannot decode (for example MXF, ProRes, MJPEG). The player runs in
  a separate program, `mam-player-worker.exe` (part of velio.), which loads the DLL at run time (`LoadLibrary`);
  neither velio. nor the worker is linked against it at build time.
- Version: mpv v0.42.0, the official release asset `libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip` from
  <https://github.com/mpv-player/mpv/releases/tag/v0.42.0> (SHA-256
  `8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce`). Only `libmpv-2.dll` and `LICENSE.LGPL` of that
  archive are installed (the header files, the import library and the two libbluray `.jar` files are not).
- License: **GNU Lesser General Public License v3 or later** for `libmpv-2.dll` as distributed. mpv itself is built with
  `-Dgpl=false` (GPL-only files excluded) and is LGPL-2.1-or-later; the FFmpeg code statically contained in the DLL is
  LGPL-3.0-or-later (configured with `--enable-version3`, without `--enable-gpl` / `--enable-nonfree`; the DLL reports
  "libavcodec license: LGPL version 3 or later"), so the combined library is distributed under LGPL v3 or later.
  Full texts: `libmpv\LICENSE.LGPL` (LGPL v2.1, as shipped by mpv) and the LGPL v3 text in `THIRD_PARTY_LICENSES.md`
  (same folder as this file). mpv is copyright (c) the mpv developers; see 1.3.1 for the libraries built into it.
  **Known open issue (libzvbi)**: see the notes on 1.3.1.
- **Source code**: mpv <https://github.com/mpv-player/mpv/tree/v0.42.0>
  (<https://github.com/mpv-player/mpv/archive/refs/tags/v0.42.0.tar.gz>). The DLL was built by mpv's release workflow
  (`.github/workflows/build.yml`, job `mingw64-full`, `ci/build-mingw64-full.sh` at the tag) inside the
  `ghcr.io/btbn/ffmpeg-builds/win64-lgpl` image of [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)
  (build scripts `scripts.d/*.sh`, which pin each dependency to a git commit or tag); the FFmpeg inside is
  `git-2026-11-04-baa68e6` (<https://github.com/FFmpeg/FFmpeg/commit/baa68e6>, libavcodec 63.26.100). The unmodified
  binary comes from that official release. You can also request the complete corresponding source code of libmpv,
  FFmpeg and the libraries listed in 1.3.1 and 1.1.1 from the velio. distributor by opening an issue at
  <https://github.com/laypauls/velio-mam-desktop/issues>
  (written offer valid for at least three years after the release).
- **Replacing the library (LGPL relinking)**: `libmpv-2.dll` is a separate, dynamically loaded file. You may replace it
  with your own build of a compatible libmpv (a `libmpv-2.dll` exporting the mpv client API, version 2.x) by copying it
  over `libmpv\libmpv-2.dll` in the install folder, or point the environment variable `MAM_LIBMPV_DIR` to another
  folder that contains your `libmpv-2.dll`. Nothing in velio. has to be rebuilt or relinked for this.

#### 1.3.1 Libraries built into libmpv-2.dll

The list is taken from the build configuration embedded in the shipped DLL (the FFmpeg `configure` line and the mpv
`meson` options are stored in it), mpv's CI build script at the tag, and the BtbN dependency scripts. "Checked" says what
was confirmed in the DLL: "version" = a version string was found; "code" = library code or messages were found but no
version string; "no" = based on the build scripts only. The libraries that FFmpeg links through the BtbN image are listed
in the table of 1.1.1 (same image and scripts; the enabled set is the same except for the differences below), so they
are not repeated here; their versions in libmpv are the commits pinned by the BtbN scripts at the date of the mpv
v0.42.0 build (2026-11-04) and can be newer than the commits listed in 1.1.1.

| Library | Version | Used by | License (SPDX) | Copyright | Checked | Source |
|---|---|---|---|---|---|---|
| mpv (libmpv client API and player core) | v0.42.0 | `libmpv-2.dll` | LGPL-2.1-or-later (built with `-Dgpl=false`; some files are MIT / ISC / BSD, see mpv's `Copyright` file) | The mpv developers (mpv, mplayer2 and MPlayer contributors) | version (`mpv v0.42.0`) | <https://github.com/mpv-player/mpv/tree/v0.42.0> |
| FFmpeg (libavcodec, libavformat, libavfilter, libavutil, libswscale, libswresample) | git-2026-11-04-baa68e6 | `libmpv-2.dll` | LGPL-3.0-or-later | The FFmpeg developers | version (Lavc 63.26.100, "LGPL version 3 or later") | <https://github.com/FFmpeg/FFmpeg/commit/baa68e6> |
| LuaJIT | 2.1.1793383622 (branch v2.1) | `libmpv-2.dll` | MIT | Mike Pall | version | <https://github.com/LuaJIT/LuaJIT> |
| mujs | 1.3.8 | `libmpv-2.dll` | ISC | Artifex Software, Inc. | version | <https://github.com/ccxvii/mujs> |
| libarchive | 3.9.0dev | `libmpv-2.dll` | BSD-2-Clause (some files carry other permissive terms, see its `COPYING`) | Tim Kientzle and others | version | <https://github.com/libarchive/libarchive> |
| uchardet | 0.0.8 | `libmpv-2.dll` | MPL-1.1 OR GPL-2.0-or-later OR LGPL-2.1-or-later (used under MPL-1.1) | Netscape Communications Corporation and the uchardet contributors | code | <https://gitlab.freedesktop.org/uchardet/uchardet> |
| libsixel | git master at build time | `libmpv-2.dll` | MIT | Hayaki Saito | no | <https://github.com/saitoha/libsixel> |
| subrandr (statically includes its Rust crates, not audited one by one) | git master at build time | `libmpv-2.dll` | MPL-2.0 | The subrandr authors | code | <https://github.com/afishhh/subrandr> |
| libcurl | 8.23.1-DEV | `libmpv-2.dll` | curl | Daniel Stenberg, <daniel@haxx.se>, and many contributors | version | <https://github.com/curl/curl> |
| OpenSSL (used by libcurl) | 3.6.5 | `libmpv-2.dll` | Apache-2.0 | The OpenSSL Project Authors | version | <https://github.com/openssl/openssl> |
| nghttp2 (used by libcurl) | BtbN scripts.d pin | `libmpv-2.dll` | MIT | Tatsuhiro Tsujikawa and the nghttp2 contributors | code | <https://github.com/nghttp2/nghttp2> |
| nghttp3 / ngtcp2 (HTTP/3 for libcurl) | BtbN scripts.d pin | `libmpv-2.dll` | MIT | Tatsuhiro Tsujikawa and the contributors | no | <https://github.com/ngtcp2/nghttp3>, <https://github.com/ngtcp2/ngtcp2> |
| libpsl (used by libcurl) | BtbN scripts.d pin | `libmpv-2.dll` | MIT | Tim Ruehsen and the libpsl contributors | code | <https://github.com/rockdaboot/libpsl> |
| kissfft (FFT inside Chromaprint) | bundled with Chromaprint | `libmpv-2.dll` | BSD-3-Clause | Mark Borgerding | no | <https://github.com/mborgerding/kissfft> |
| VapourSynth API headers (compile time only; VapourSynth itself is not bundled and is looked up on the user's PC at run time) | R82 | `libmpv-2.dll` | LGPL-2.1-or-later | Fredrik Mellbin and the VapourSynth contributors | no | <https://github.com/vapoursynth/vapoursynth> |
| C++/WinRT (headers generated at build time for mpv's Windows media-key integration) | 2.0.250303.1 | `libmpv-2.dll` | MIT | Microsoft Corporation | no | <https://github.com/microsoft/cppwinrt> |

Notes on 1.3.1:

- **Differences from the FFmpeg configuration of 1.1.1**: libmpv's FFmpeg is configured with `--disable-librav1e`,
  `--disable-librsvg`, `--disable-openal` and `--enable-lcms2`, `--enable-libcurl`, `--enable-vapoursynth`, and without
  FFmpeg's own `--enable-libshaderc` (shaderc, SPIRV-Cross, libplacebo, libass, Little CMS, zimg, libbluray, the Vulkan
  loader and FriBidi / HarfBuzz / FreeType are used by mpv directly: `-Dshaderc=enabled -Dspirv-cross=enabled
  -Dlcms2=enabled -Dzimg=enabled -Dlibbluray=enabled -Dvulkan=enabled -Dsubrandr=enabled -Duchardet=enabled
  -Dlibarchive=enabled -Dlibcurl=enabled -Dsixel=enabled`). rav1e and librsvg (and so the Rust crates, GLib, cairo and Pango
  that librsvg brings) are therefore not part of this DLL; their rows in 1.1.1 apply to the FFmpeg programs only.
- **FFTW is not present.** The BtbN scripts build Chromaprint with its internal kissfft since 2026-10-28; the string
  "fftw" does not occur in `libmpv-2.dll` (the FFmpeg DLL of 1.1 contains over a thousand FFTW strings). The GPL-2.0-or-later
  FFTW therefore does not apply to libmpv.
- **libzvbi (open issue).** The DLL is configured with `--enable-libzvbi` (the Teletext decoder `libzvbi-teletextdec` of
  libavcodec). libzvbi's `src/packet-830.c` and `src/pdc.c` are licensed GPL-2.0-only (its `COPYING.md`), and the zvbi
  Teletext packet decoder that FFmpeg's decoder calls (`vbi_decode` -> `packet.c`) calls into `packet-830.c`, so GPL-2.0-only
  object code is considered to be linked into `libmpv-2.dll`; the symbols are stripped and this was not confirmed in the
  binary itself. GPL-2.0-only code cannot be combined with the LGPL-3.0-or-later FFmpeg into an LGPL-3.0 work. This is
  the upstream issue [BtbN/FFmpeg-Builds#694](https://github.com/BtbN/FFmpeg-Builds/issues/694), fixed on 2026-11-07 by
  removing libzvbi from the LGPL and GPL images; the v0.42.0 binary was built before that fix. velio. does not expose
  Teletext decoding. The licensing of this component must be resolved before release (for example with a libmpv built without
  `--enable-libzvbi`); this paragraph describes the shipped binary until then.
- The mingw-w64 runtime and winpthreads, and the GCC runtime libraries (GPL-3.0-or-later WITH GCC-exception-3.1),
  may also be statically included, as in 1.1.1; their versions are not pinned in the build scripts.
- Portions of this software are copyright (c) The FreeType Project (<https://freetype.org>). All rights reserved.

## 2. Libraries compiled into the application

| Component | Version | License | Notes |
|---|---|---|---|
| ONNX Runtime (via `ort` / `ort-sys`) | 1.31 (ort 2.0.0-rc.15) | MIT (onnxruntime, Copyright (c) Microsoft Corporation); `ort` MIT OR Apache-2.0 | statically linked (CPU provider). onnxruntime's own third-party notices (`ThirdPartyNotices.txt`) are reproduced in `THIRD_PARTY_LICENSES.md` |
| Lindera (+ `lindera-ko-dic`, `lindera-dictionary`) | 6.2.0 | MIT | Korean morphological analysis |
| **mecab-ko-dic** dictionary data (embedded in the binary by `lindera-ko-dic`) | 2.1.1-20180720 | **Apache-2.0** | see section 4 |
| Tantivy | 0.27 | MIT | full-text index |
| rquickjs, rquickjs-core, rquickjs-sys | 0.14.0 | MIT (Copyright (c) 2020 Mees Delzenne; (c) 2025 Rquickjs Contributors) | Rust bindings; linked only into the plugin worker `mam-plugin-worker.exe` (install folder), which runs plugin scripts. <https://github.com/DelSkayn/rquickjs> |
| QuickJS-NG (vendored in `rquickjs-sys` 0.14.0) | 0.16.2 | MIT | JavaScript engine, statically linked into `mam-plugin-worker.exe`; unmodified, see 2.1. <https://github.com/quickjs-ng/quickjs> |
| SQLite (via `rusqlite` / `libsqlite3-sys`) | bundled | public domain (SQLite) / MIT (bindings) | |
| libwebp (via `libwebp-sys` / `webp`) | bundled | BSD-3-Clause (libwebp) / MIT (bindings) | thumbnails |
| `image`, `kamadak-exif`, `tokenizers`, `ndarray`, `windows` | see `THIRD_PARTY_LICENSES.md` | MIT and/or Apache-2.0 (`tokenizers` Apache-2.0, `kamadak-exif` BSD-2-Clause) | |
| Tauri 2, wry, tao, WebView2 bindings | 2.x | MIT OR Apache-2.0 | The Microsoft Edge WebView2 Runtime is a separate Microsoft component installed on Windows |
| React, TanStack Query/Virtual, Radix UI, shadcn/ui components, lucide-react, i18next, Tailwind CSS, sonner, ... | see `THIRD_PARTY_LICENSES.md` | MIT / ISC (lucide-react) | npm production dependencies |
| Inter variable font (`@fontsource-variable/inter`) | 5.x | SIL Open Font License 1.1 | <https://github.com/rsms/inter/blob/master/LICENSE.txt> |
| MPL-2.0 crates: `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`, `htmlescape` (MPL-2.0 option) | see `THIRD_PARTY_LICENSES.md` | MPL-2.0 | unmodified; source: crates.io, <https://github.com/servo/rust-cssparser>, <https://github.com/servo/stylo> |
| `webpki-roots` | 1.x | CDLA-Permissive-2.0 | CA root data |

Rust crates under "Apache-2.0 OR MIT" (the vast majority) are used under the MIT option. The copyright notices and
license texts of all compiled-in Rust crates and npm packages are in `THIRD_PARTY_LICENSES.md` (same folder; also shown
in Settings > Open-source licenses).

### 2.1 QuickJS-NG (JavaScript engine in the plugin worker) - MIT

`mam-plugin-worker.exe` statically contains QuickJS-NG v0.16.2, the maintained fork of Fabrice Bellard's and Charlie
Gordon's QuickJS, as vendored unmodified by the `rquickjs-sys` 0.14.0 crate (`quickjs/LICENSE`). Plugin scripts run
inside it with no file, network or process access.

```
The MIT License (MIT)

Copyright (c) 2017-2026 Fabrice Bellard
Copyright (c) 2017-2024 Charlie Gordon
Copyright (c) 2023-2026 Ben Noordhuis
Copyright (c) 2023-2026 Saúl Ibarra Corretgé

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit
persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the
Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

The `rquickjs` bindings are under the MIT License, Copyright (c) 2020 Mees Delzenne, Copyright (c) 2025 Rquickjs
Contributors (full text in `THIRD_PARTY_LICENSES.md` after its next regeneration).

## 3. AI models (not bundled - downloaded on first use, SHA-256 verified)

The installer does not contain any model. When the user enables the feature, the application downloads the files
listed in the model list built into the application (each download URL is pinned to a fixed revision and verified by
SHA-256) into the user's data folder (`MAMData\models`).

| Model id | Source | License |
|---|---|---|
| `clip-vit-b32-vision` (CLIP ViT-B/32 image encoder, ONNX fp32) | <https://huggingface.co/Xenova/clip-vit-base-patch32> | MIT (OpenAI CLIP) |
| `clip-b32-multilingual-text` (clip-ViT-B-32-multilingual-v1 text encoder, ONNX uint8 + Dense) | <https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1> | Apache-2.0 |
| `multilingual-e5-small` (ONNX) | <https://huggingface.co/intfloat/multilingual-e5-small> | MIT |
| `whisper-base`, `whisper-small` (ggml) | <https://huggingface.co/ggerganov/whisper.cpp> (OpenAI Whisper weights) | MIT |

The model licenses above are the values recorded in the model list; they were not independently audited against the
upstream model cards for every revision.

## 4. mecab-ko-dic (Apache-2.0)

This software includes a binary version of data from mecab-ko-dic-2.1.1-20180720, which can be obtained from
<https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz>
(project: <https://bitbucket.org/eunjeon/mecab-ko-dic>). It is distributed under the Apache License, Version 2.0,
reproduced below (this text also applies to the other Apache-2.0 components listed in this file).

```
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
