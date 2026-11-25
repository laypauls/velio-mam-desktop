//! P1-5 시험 샘플 생성기 — 지정 폴더(임시 디렉터리 권장)에 §2.2 의 모든 형식 샘플을 만든다.
//!
//! ```text
//! cargo run -p mam-media --example gen_samples -- <출력 폴더>
//! ```
//!
//! 생성 도구: 이미지는 `image` crate(+ImageMagick `magick`: AVIF/HEIC/대형 JPEG), 영상·음성은 `ffmpeg`.
//! 이 도구들은 **샘플 생성에만** 쓰이며 라이브러리의 런타임 의존이 아니다. 없는 도구가 필요한 샘플은
//! `SKIP` 으로 출력하고 넘어간다. 실제 카메라 RAW(CR2/NEF/ARW)와 방송 장비 MXF 는 구할 수 없어
//! 합성 샘플(TIFF 구조 / FFmpeg MXF)로 대체한다 — 호환표에 그대로 표기할 것.

#[path = "../tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::{ImageFormat, Rgb, RgbImage};

fn run(tool: &str, args: &[&str]) -> bool {
    Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Gen {
    out: PathBuf,
    ok: usize,
    skipped: Vec<String>,
}

impl Gen {
    fn p(&self, name: &str) -> PathBuf {
        self.out.join(name)
    }

    fn done(&mut self, name: &str, ok: bool) {
        if ok
            && self.p(name).is_file()
            && fs::metadata(self.p(name))
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            self.ok += 1;
            println!("OK   {name}");
        } else {
            let _ = fs::remove_file(self.p(name));
            println!("SKIP {name} (generator tool missing or encoder unavailable)");
            self.skipped.push(name.to_string());
        }
    }

    fn write(&mut self, name: &str, bytes: &[u8]) {
        fs::write(self.p(name), bytes).unwrap();
        self.done(name, true);
    }

    /// `ffmpeg -y <src...> <opts...> <name>`
    fn ff(&mut self, name: &str, src: &[&str], opts: &[&str]) {
        let target = self.p(name);
        let mut args = vec!["-hide_banner", "-loglevel", "error", "-y"];
        args.extend_from_slice(src);
        args.extend_from_slice(opts);
        args.push(target.to_str().unwrap());
        let ok = run("ffmpeg", &args);
        self.done(name, ok);
    }
}

const V: [&str; 4] = [
    "-f",
    "lavfi",
    "-i",
    "testsrc=size=320x240:rate=25:duration=2",
];
const A: [&str; 4] = ["-f", "lavfi", "-i", "sine=frequency=440:duration=2"];

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: gen_samples <output dir>"),
    );
    fs::create_dir_all(&out).unwrap();
    let mut g = Gen {
        out,
        ok: 0,
        skipped: vec![],
    };

    // ---- 이미지 (image crate) ----
    let base = common::red_blue(640, 480);
    g.write(
        "sample.jpg",
        &common::jpeg_with_exif(&base, 6, "2024:05:01 12:34:56"),
    );
    for (name, fmt) in [
        ("sample.png", ImageFormat::Png),
        ("sample.gif", ImageFormat::Gif),
        ("sample.bmp", ImageFormat::Bmp),
        ("sample.tiff", ImageFormat::Tiff),
    ] {
        g.write(name, &common::encode(&base, fmt));
    }
    {
        // WebP: image crate 는 무손실 인코딩만 지원
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(base.clone())
            .write_to(&mut buf, ImageFormat::WebP)
            .unwrap();
        g.write("sample.webp", buf.get_ref());
    }

    // ---- 이미지 (ImageMagick: AVIF / HEIC / 대형) ----
    let png = g.p("sample.png");
    let png = png.to_str().unwrap().to_string();
    let avif = g.p("sample.avif");
    let ok = run("magick", &[&png, avif.to_str().unwrap()]);
    g.done("sample.avif", ok);
    // HEIC: x265 로 인트라 1프레임 -> 직접 HEIF 컨테이너 조립 (ImageMagick 은 HEIC 쓰기 불가)
    let hevc = g.p("_tmp.hevc");
    let made = run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=1:duration=1",
            "-frames:v",
            "1",
            "-c:v",
            "libx265",
            "-x265-params",
            "keyint=1:repeat-headers=1:log-level=none",
            "-f",
            "hevc",
            hevc.to_str().unwrap(),
        ],
    );
    let heic = made
        .then(|| fs::read(&hevc).ok())
        .flatten()
        .and_then(|b| build_heic(&b, 320, 240));
    let _ = fs::remove_file(&hevc);
    match heic {
        Some(b) => g.write("sample.heic", &b),
        None => g.done("sample.heic", false),
    }
    let big = g.p("big_12000x9000.jpg");
    let ok = run(
        "magick",
        &[
            "-size",
            "12000x9000",
            "gradient:red-blue",
            big.to_str().unwrap(),
        ],
    );
    g.done("big_12000x9000.jpg", ok);
    let mid = g.p("mid_6000x4000.jpg");
    let ok = run(
        "magick",
        &[
            "-size",
            "6000x4000",
            "gradient:red-blue",
            mid.to_str().unwrap(),
        ],
    );
    g.done("mid_6000x4000.jpg", ok);

    // ---- RAW (합성 TIFF 구조 — 실제 카메라 파일 아님) ----
    let preview = common::encode(&base, ImageFormat::Jpeg);
    let dng = common::synthetic_dng(&preview, 6);
    for name in [
        "synthetic.dng",
        "synthetic.cr2",
        "synthetic.nef",
        "synthetic.arw",
    ] {
        g.write(name, &dng);
    }

    // ---- 동영상 (ffmpeg) ----
    let h264 = [
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "veryfast",
        "-c:a",
        "aac",
        "-shortest",
    ];
    let va = |g: &mut Gen, name: &str, opts: &[&str]| {
        let mut src: Vec<&str> = V.to_vec();
        src.extend_from_slice(&A);
        g.ff(name, &src, opts);
    };
    va(&mut g, "video_h264.mp4", &h264);
    va(&mut g, "video_h264.mov", &h264);
    va(&mut g, "video_h264.mkv", &h264);
    va(
        &mut g,
        "video_mpeg4_mp3.avi",
        &["-c:v", "mpeg4", "-c:a", "libmp3lame", "-shortest"],
    );
    va(
        &mut g,
        "video_wmv2.wmv",
        &["-c:v", "wmv2", "-c:a", "wmav2", "-shortest"],
    );
    va(
        &mut g,
        "video_vp9.webm",
        &[
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "200k",
            "-c:a",
            "libopus",
            "-shortest",
        ],
    );
    va(
        &mut g,
        "video_mpeg2.mpg",
        &["-c:v", "mpeg2video", "-c:a", "mp2", "-shortest"],
    );
    va(
        &mut g,
        "video_h264.mts",
        &[
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "veryfast",
            "-c:a",
            "ac3",
            "-f",
            "mpegts",
            "-shortest",
        ],
    );
    va(
        &mut g,
        "video_hevc.mp4",
        &[
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-tag:v",
            "hvc1",
            "-shortest",
        ],
    );
    va(
        &mut g,
        "video_av1.mp4",
        &[
            "-c:v",
            "libaom-av1",
            "-cpu-used",
            "8",
            "-b:v",
            "200k",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    va(
        &mut g,
        "video_prores.mov",
        &["-c:v", "prores_ks", "-c:a", "pcm_s16le", "-shortest"],
    );
    va(
        &mut g,
        "video_mpeg2_pcm.mxf",
        &[
            "-c:v",
            "mpeg2video",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "48000",
            "-f",
            "mxf",
            "-shortest",
        ],
    );
    // XDCAM HD 422 흉내 (MPEG-2 4:2:2 1080i, 50Mbps) — 실제 방송 장비 파일이 아니다
    let hd = [
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=1920x1080:rate=25:duration=1",
    ];
    let mut src: Vec<&str> = hd.to_vec();
    src.extend_from_slice(&["-f", "lavfi", "-i", "sine=frequency=440:duration=1"]);
    g.ff(
        "video_xdcam_like.mxf",
        &src,
        &[
            "-c:v",
            "mpeg2video",
            "-pix_fmt",
            "yuv422p",
            "-b:v",
            "50M",
            "-minrate",
            "50M",
            "-maxrate",
            "50M",
            "-bufsize",
            "17M",
            "-flags",
            "+ildct+ilme",
            "-c:a",
            "pcm_s24le",
            "-ar",
            "48000",
            "-f",
            "mxf",
            "-shortest",
        ],
    );
    // DNxHD (MXF)
    g.ff(
        "video_dnxhd.mxf",
        &src,
        &[
            "-c:v",
            "dnxhd",
            "-b:v",
            "36M",
            "-pix_fmt",
            "yuv422p",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "48000",
            "-f",
            "mxf",
            "-shortest",
        ],
    );
    // 회전 메타데이터 (세로 촬영 스마트폰 영상 흉내)
    let h = g.p("video_h264.mp4");
    if h.is_file() {
        g.ff(
            "video_rotated90.mp4",
            &["-display_rotation", "90", "-i", h.to_str().unwrap()],
            &["-c", "copy"],
        );
    }
    // M2TS: 188바이트 TS 패킷마다 4바이트 타임코드를 붙인 BDAV 형태로 변환
    if let Ok(ts) = fs::read(g.p("video_h264.mts")) {
        let mut m2ts = Vec::with_capacity(ts.len() / 188 * 192);
        for pkt in ts.chunks_exact(188) {
            m2ts.extend_from_slice(&[0, 0, 0, 0]);
            m2ts.extend_from_slice(pkt);
        }
        g.write("video_h264.m2ts", &m2ts);
    }

    // ---- 음성 (ffmpeg) ----
    g.ff("audio.mp3", &A, &["-c:a", "libmp3lame"]);
    g.ff("audio.wav", &A, &["-c:a", "pcm_s16le"]);
    g.ff("audio.aac", &A, &["-c:a", "aac", "-f", "adts"]);
    g.ff("audio.m4a", &A, &["-c:a", "aac"]);
    g.ff("audio.flac", &A, &["-c:a", "flac"]);
    g.ff("audio.ogg", &A, &["-c:a", "libvorbis"]);
    g.ff("audio.opus", &A, &["-c:a", "libopus"]);
    g.ff("audio.wma", &A, &["-c:a", "wmav2"]);
    g.ff("audio.aiff", &A, &["-c:a", "pcm_s16be", "-f", "aiff"]);

    // ---- 경계 케이스 ----
    let k = g.out.join("한글 폴더 (복사본)");
    fs::create_dir_all(&k).unwrap();
    if let Ok(b) = fs::read(g.p("video_h264.mp4")) {
        fs::write(k.join("영상 파일 #1 [최종].mp4"), &b).unwrap();
        fs::write(g.p("edge_zero.mp4"), b"").unwrap();
        fs::write(g.p("edge_truncated.mp4"), &b[..b.len() / 3]).unwrap();
    }
    fs::write(g.p("edge_zero.jpg"), b"").unwrap();
    fs::write(g.p("edge_disguised.jpg"), b"%PDF-1.7 not an image").unwrap();
    fs::write(g.p("edge_corrupt.png"), b"\x89PNG\r\n\x1a\ngarbage").unwrap();
    fs::write(g.p("doc.pdf"), b"%PDF-1.7").unwrap();
    let tiny = RgbImage::from_pixel(1, 1, Rgb([1, 2, 3]));
    g.write("edge_1x1.png", &common::encode(&tiny, ImageFormat::Png));

    println!(
        "\n{} generated, {} skipped: {:?}",
        g.ok,
        g.skipped.len(),
        g.skipped
    );
    let _: &Path = &g.out;
}

// ---- 단일 아이템 HEIC 조립 (HEVC 인트라 프레임 1장을 HEIF 컨테이너에 넣는다) ----
// ImageMagick 의 libheif 는 디코드 전용이라 HEIC 를 만들 수 없어 직접 조립한다.
// iPhone 이 만드는 그리드(타일) HEIC 가 아니라 **단일 이미지 아이템**이다 — 그리드는 미시험.

fn bx(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(typ);
    v.extend_from_slice(payload);
    v
}

fn fbx(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    bx(typ, &[&[0u8; 4][..], payload].concat())
}

/// Annex-B 바이트스트림을 NAL 단위로 나눈다.
fn split_nals(s: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= s.len() {
        if s[i] == 0 && s[i + 1] == 0 && s[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    (0..starts.len())
        .map(|k| {
            let end = starts.get(k + 1).map(|n| n - 3).unwrap_or(s.len());
            let mut nal = &s[starts[k]..end];
            while nal.last() == Some(&0) {
                nal = &nal[..nal.len() - 1];
            }
            nal
        })
        .collect()
}

fn build_heic(hevc: &[u8], width: u16, height: u16) -> Option<Vec<u8>> {
    let nals = split_nals(hevc);
    let of = |t: u8| nals.iter().find(|n| (n[0] >> 1) & 0x3F == t).copied();
    let (vps, sps, pps) = (of(32)?, of(33)?, of(34)?);
    let slice = nals
        .iter()
        .find(|n| matches!((n[0] >> 1) & 0x3F, 19..=21))?;
    // SPS 의 profile_tier_level (2바이트 NAL 헤더 + 1바이트 이후 12바이트) — 에뮬레이션 방지 바이트 제거 후 읽는다
    let mut rbsp = Vec::new();
    let mut zeros = 0;
    for &b in &sps[2..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    let ptl = rbsp.get(1..13)?;
    let mut hvcc = vec![1];
    hvcc.extend_from_slice(ptl); // profile 바이트, 호환 플래그 4, 제약 플래그 6, level
    hvcc.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F, 3]);
    for (t, n) in [(0xA0u8, vps), (0xA1, sps), (0xA2, pps)] {
        hvcc.push(t);
        hvcc.extend_from_slice(&1u16.to_be_bytes());
        hvcc.extend_from_slice(&(n.len() as u16).to_be_bytes());
        hvcc.extend_from_slice(n);
    }
    let mut sample = (slice.len() as u32).to_be_bytes().to_vec();
    sample.extend_from_slice(slice);

    let ispe = fbx(
        b"ispe",
        &[(width as u32).to_be_bytes(), (height as u32).to_be_bytes()].concat(),
    );
    let ipco = bx(b"ipco", &[bx(b"hvcC", &hvcc), ispe].concat());
    let ipma = fbx(
        b"ipma",
        &[
            &1u32.to_be_bytes()[..],
            &1u16.to_be_bytes(),
            &[2, 0x81, 0x02],
        ]
        .concat(),
    );
    let iprp = bx(b"iprp", &[ipco, ipma].concat());
    let hdlr = fbx(
        b"hdlr",
        &[&[0u8; 4][..], b"pict", &[0u8; 12], &[0]].concat(),
    );
    let pitm = fbx(b"pitm", &1u16.to_be_bytes());
    let infe = {
        let mut body = vec![2u8, 0, 0, 0]; // version=2, flags=0
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(b"hvc1");
        body.push(0);
        bx(b"infe", &body)
    };
    let iinf = fbx(b"iinf", &[&1u16.to_be_bytes()[..], &infe].concat());
    let ftyp = bx(b"ftyp", &[&b"heic"[..], &[0; 4], b"mif1", b"heic"].concat());
    // iloc 은 mdat 의 절대 오프셋이 필요하다: 크기는 오프셋 값과 무관하므로 두 번 만든다.
    let iloc = |off: u32| {
        let mut p = vec![0x44, 0x00]; // offset_size=4, length_size=4, base_offset_size=0, index_size=0
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes()); // item_ID
        p.extend_from_slice(&0u16.to_be_bytes()); // data_reference_index
        p.extend_from_slice(&1u16.to_be_bytes()); // extent_count
        p.extend_from_slice(&off.to_be_bytes());
        p.extend_from_slice(&(sample.len() as u32).to_be_bytes());
        fbx(b"iloc", &p)
    };
    let meta_len = |off| {
        fbx(
            b"meta",
            &[
                hdlr.clone(),
                pitm.clone(),
                iloc(off),
                iinf.clone(),
                iprp.clone(),
            ]
            .concat(),
        )
        .len()
    };
    let off = (ftyp.len() + meta_len(0) + 8) as u32;
    let meta = fbx(b"meta", &[hdlr, pitm, iloc(off), iinf, iprp].concat());
    Some([ftyp, meta, bx(b"mdat", &sample)].concat())
}
