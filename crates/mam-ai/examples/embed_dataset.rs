//! COCO val2017(5,000장) + 합성 이미지의 **실제 CLIP 임베딩**을 파일로 만든다 (P1-3 재측정·검색 평가 입력).
//!
//! `embed_dataset --vision <onnx> --text <onnx> --tokenizer <json> --dense <safetensors> --coco <dir> --out <dir> [--threads 6] [--limit N] [--synth 3000]`
//!
//! `<coco>` 에는 `val2017/*.jpg`, `annotations/captions_val2017.json`, `annotations/instances_val2017.json` 이 있어야 한다.
//! 이미지 1장당 3 벡터(원본, 좌우 반전, 중앙 70% 크롭)를 만든다 - 영상 대표 프레임처럼 서로 가까운 벡터군을 흉내낸다.
//! 이 파일들은 측정 전용이며 저장소에 넣지 않는다 (COCO 이미지는 Flickr 개별 라이선스).

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use common::synth::{Rng, random_image};
use common::{Args, fail};
use image::DynamicImage;
use mam_ai::local::embed::image_pre::{CLIP_TENSOR_LEN, decode_image, preprocess_clip};
use mam_ai::local::embed::{ClipTextEmbedder, EmbedConfig, ImageEmbedder};
use serde::Deserialize;

const COCO_EN: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];
const COCO_KO: [&str; 80] = [
    "사람",
    "자전거",
    "자동차",
    "오토바이",
    "비행기",
    "버스",
    "기차",
    "트럭",
    "보트",
    "신호등",
    "소화전",
    "정지 표지판",
    "주차 미터기",
    "벤치",
    "새",
    "고양이",
    "강아지",
    "말",
    "양",
    "소",
    "코끼리",
    "곰",
    "얼룩말",
    "기린",
    "배낭",
    "우산",
    "핸드백",
    "넥타이",
    "여행 가방",
    "프리스비 원반",
    "스키",
    "스노보드",
    "공",
    "연",
    "야구 방망이",
    "야구 글러브",
    "스케이트보드",
    "서핑보드",
    "테니스 라켓",
    "병",
    "와인잔",
    "컵",
    "포크",
    "칼",
    "숟가락",
    "그릇",
    "바나나",
    "사과",
    "샌드위치",
    "오렌지",
    "브로콜리",
    "당근",
    "핫도그",
    "피자",
    "도넛",
    "케이크",
    "의자",
    "소파",
    "화분",
    "침대",
    "식탁",
    "변기",
    "텔레비전",
    "노트북",
    "마우스",
    "리모컨",
    "키보드",
    "휴대폰",
    "전자레인지",
    "오븐",
    "토스터",
    "싱크대",
    "냉장고",
    "책",
    "시계",
    "꽃병",
    "가위",
    "곰 인형",
    "헤어드라이어",
    "칫솔",
];

#[derive(Deserialize)]
struct Captions {
    annotations: Vec<CapAnn>,
}
#[derive(Deserialize)]
struct CapAnn {
    image_id: u64,
    caption: String,
}
#[derive(Deserialize)]
struct Instances {
    images: Vec<ImgInfo>,
    annotations: Vec<InstAnn>,
    categories: Vec<Cat>,
}
#[derive(Deserialize)]
struct ImgInfo {
    id: u64,
    width: f64,
    height: f64,
}
#[derive(Deserialize)]
struct InstAnn {
    image_id: u64,
    category_id: u32,
    area: f64,
}
#[derive(Deserialize)]
struct Cat {
    id: u32,
    name: String,
}

fn write_f32(path: &Path, rows: &[Vec<f32>]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).expect("create"));
    for r in rows {
        for x in r {
            f.write_all(&x.to_le_bytes()).expect("write");
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let s =
        std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    serde_json::from_str(&s).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())))
}

fn main() {
    let a = Args::parse();
    let threads: usize = a.num("threads", 6);
    let cfg = EmbedConfig {
        threads,
        batch_size: 8,
    };
    let coco = PathBuf::from(a.require("coco"));
    let out = PathBuf::from(a.require("out"));
    std::fs::create_dir_all(&out).expect("out dir");
    let limit: usize = a.num("limit", usize::MAX);
    let n_synth: usize = a.num("synth", 3000);

    let vision = ImageEmbedder::load(Path::new(a.require("vision")), cfg)
        .unwrap_or_else(|e| fail(&e.to_string()));
    let text = ClipTextEmbedder::load(
        Path::new(a.require("text")),
        Path::new(a.require("tokenizer")),
        Path::new(a.require("dense")),
        cfg,
    )
    .unwrap_or_else(|e| fail(&e.to_string()));

    // 주석
    let caps: Captions = read_json(&coco.join("annotations/captions_val2017.json"));
    let inst: Instances = read_json(&coco.join("annotations/instances_val2017.json"));
    let mut cats = inst.categories;
    cats.sort_by_key(|c| c.id);
    for (c, en) in cats.iter().zip(COCO_EN) {
        if c.name != en {
            fail(&format!("category order mismatch: {} vs {en}", c.name));
        }
    }
    let mut first_caption: HashMap<u64, String> = HashMap::new();
    for c in caps.annotations {
        first_caption
            .entry(c.image_id)
            .or_insert(c.caption.trim().to_string());
    }
    let mut ids: Vec<u64> = inst.images.iter().map(|i| i.id).collect();
    ids.sort_unstable();
    ids.truncate(limit);
    let sizes: HashMap<u64, f64> = inst
        .images
        .iter()
        .map(|i| (i.id, i.width * i.height))
        .collect();
    // 이미지별 카테고리 면적 비율
    let mut cover: HashMap<(u64, u32), f64> = HashMap::new();
    for an in &inst.annotations {
        *cover.entry((an.image_id, an.category_id)).or_default() += an.area / sizes[&an.image_id];
    }

    // 이미지 임베딩 (디코딩 스레드 + 추론)
    let started = Instant::now();
    let (tx, rx) = mpsc::sync_channel::<(Vec<(String, u64)>, Vec<f32>)>(4);
    let ids_p = ids.clone();
    let dir = coco.join("val2017");
    let producer = std::thread::spawn(move || {
        let mut meta = Vec::new();
        let mut px: Vec<f32> = Vec::new();
        let flush = |meta: &mut Vec<(String, u64)>, px: &mut Vec<f32>| {
            if !meta.is_empty() {
                tx.send((std::mem::take(meta), std::mem::take(px))).ok();
            }
        };
        for id in ids_p {
            let path = dir.join(format!("{id:012}.jpg"));
            let img = decode_image(&path).unwrap_or_else(|e| fail(&e.to_string()));
            let (w, h) = (img.width(), img.height());
            let crop = img.crop_imm(w * 15 / 100, h * 15 / 100, w * 70 / 100, h * 70 / 100);
            for (kind, view) in [("orig", img.clone()), ("flip", img.fliph()), ("crop", crop)] {
                px.extend(preprocess_clip(&view));
                meta.push((kind.to_string(), id));
                if meta.len() == 8 {
                    flush(&mut meta, &mut px);
                }
            }
        }
        // 합성 이미지
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for i in 0..n_synth {
            let view = DynamicImage::ImageRgb8(random_image(&mut rng));
            px.extend(preprocess_clip(&view));
            meta.push(("synth".to_string(), i as u64));
            if meta.len() == 8 {
                flush(&mut meta, &mut px);
            }
        }
        flush(&mut meta, &mut px);
    });

    let mut vectors: Vec<Vec<f32>> = Vec::new();
    let mut metas: Vec<(String, u64)> = Vec::new();
    for (meta, px) in rx {
        debug_assert_eq!(px.len(), meta.len() * CLIP_TENSOR_LEN);
        vectors.extend(vision.run_pixels(meta.len(), px).expect("infer"));
        metas.extend(meta);
        if vectors.len() % 800 < 8 {
            let s = started.elapsed().as_secs_f64();
            eprintln!(
                "  {} vectors, {:.1}/s",
                vectors.len(),
                vectors.len() as f64 / s
            );
        }
    }
    producer.join().expect("producer");
    let img_secs = started.elapsed().as_secs_f64();
    println!(
        "image vectors: {} in {:.0}s ({:.1}/s, threads={threads}, decode in a parallel thread)",
        vectors.len(),
        img_secs,
        vectors.len() as f64 / img_secs
    );
    write_f32(&out.join("image_vectors.f32"), &vectors);
    let mut f = std::fs::File::create(out.join("image_meta.tsv")).expect("meta");
    for (k, id) in &metas {
        writeln!(f, "{k}\t{id}").expect("w");
    }

    // 캡션 (영어, 이미지당 첫 1개)
    let cap_ids: Vec<u64> = ids.clone();
    let cap_texts: Vec<&str> = cap_ids
        .iter()
        .map(|id| first_caption[id].as_str())
        .collect();
    let t = Instant::now();
    let cap_vecs = text.embed_texts(&cap_texts).expect("captions");
    println!(
        "caption vectors: {} in {:.1}s",
        cap_vecs.len(),
        t.elapsed().as_secs_f64()
    );
    write_f32(&out.join("caption_vectors.f32"), &cap_vecs);
    let mut f = std::fs::File::create(out.join("caption_meta.tsv")).expect("meta");
    for (id, c) in cap_ids.iter().zip(&cap_texts) {
        writeln!(f, "{id}\t{c}").expect("w");
    }

    // 카테고리 질의 (한/영) + 관련 이미지 (면적 비율 >= 10%)
    let en_q: Vec<String> = COCO_EN
        .iter()
        .map(|n| format!("a photo of a {n}"))
        .collect();
    let ko_q: Vec<String> = COCO_KO.iter().map(|n| format!("{n} 사진")).collect();
    let en_r: Vec<&str> = en_q.iter().map(String::as_str).collect();
    let ko_r: Vec<&str> = ko_q.iter().map(String::as_str).collect();
    write_f32(
        &out.join("class_vectors_en.f32"),
        &text.embed_texts(&en_r).expect("en"),
    );
    write_f32(
        &out.join("class_vectors_ko.f32"),
        &text.embed_texts(&ko_r).expect("ko"),
    );
    let mut f = std::fs::File::create(out.join("class_relevant.tsv")).expect("rel");
    for (ci, cat) in cats.iter().enumerate() {
        let rel: Vec<String> = ids
            .iter()
            .enumerate()
            .filter(|(_, id)| cover.get(&(**id, cat.id)).is_some_and(|c| *c >= 0.10))
            .map(|(row, _)| row.to_string())
            .collect();
        writeln!(
            f,
            "{ci}\t{}\t{}\t{}",
            COCO_EN[ci],
            COCO_KO[ci],
            rel.join(",")
        )
        .expect("w");
    }
    println!("done -> {}", out.display());
}
