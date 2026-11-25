//! 합성 이미지(색·도형) - 정확도 평가와 대량 임베딩 데이터 생성용. 시드 고정 난수.

use image::{Rgb, RgbImage};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Circle,
    Square,
    Triangle,
}

pub const SHAPES: [(Shape, &str, &str); 3] = [
    (Shape::Circle, "원", "circle"),
    (Shape::Square, "사각형", "square"),
    (Shape::Triangle, "삼각형", "triangle"),
];

/// (한국어, 영어, RGB, 가능한 배경: 밝음/어두움)
pub const COLORS: [(&str, &str, [u8; 3], bool, bool); 8] = [
    ("빨간", "red", [220, 30, 30], true, true),
    ("초록", "green", [30, 160, 50], true, true),
    ("파란", "blue", [30, 60, 220], true, true),
    ("노란", "yellow", [240, 220, 40], true, true),
    ("검은", "black", [15, 15, 15], true, false),
    ("흰", "white", [250, 250, 250], false, true),
    ("주황", "orange", [240, 130, 20], true, true),
    ("보라", "purple", [130, 40, 170], true, true),
];

pub const LIGHT_BG: [u8; 3] = [235, 235, 235];
pub const DARK_BG: [u8; 3] = [50, 50, 50];

/// 중심 `(cx,cy)`, 반지름(절반 크기) `r` 의 도형을 그린다.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    shape: Shape,
    color: [u8; 3],
    bg: [u8; 3],
    w: u32,
    h: u32,
    cx: f32,
    cy: f32,
    r: f32,
) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb(bg));
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let inside = match shape {
                Shape::Circle => dx * dx + dy * dy <= r * r,
                Shape::Square => dx.abs() <= r && dy.abs() <= r,
                // 위쪽 꼭짓점의 정삼각형
                Shape::Triangle => {
                    let t = (dy + r) / (2.0 * r);
                    (0.0..=1.0).contains(&t) && dx.abs() <= r * t
                }
            };
            if inside {
                img.put_pixel(x, y, Rgb(color));
            }
        }
    }
    img
}

/// 작은 xorshift 난수 (의존성 없이 시드 고정).
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// 평가용 기본 이미지 48장 안팎: 색 x 도형 x 가능한 배경. 반환: (이미지, 색 인덱스, 도형 인덱스).
pub fn base_set() -> Vec<(RgbImage, usize, usize)> {
    let mut out = Vec::new();
    for (ci, (_, _, rgb, light, dark)) in COLORS.iter().enumerate() {
        for (si, (shape, _, _)) in SHAPES.iter().enumerate() {
            for (ok, bg) in [(*light, LIGHT_BG), (*dark, DARK_BG)] {
                if ok {
                    out.push((draw(*shape, *rgb, bg, 320, 240, 160.0, 120.0, 80.0), ci, si));
                }
            }
        }
    }
    out
}

/// 무작위 변형 1장 (위치·크기·배경 밝기·노이즈).
pub fn random_image(rng: &mut Rng) -> RgbImage {
    let (_, _, rgb, light, dark) = COLORS[rng.below(COLORS.len())];
    let (shape, _, _) = SHAPES[rng.below(SHAPES.len())];
    let use_light = match (light, dark) {
        (true, true) => rng.below(2) == 0,
        (l, _) => l,
    };
    let base = if use_light { LIGHT_BG } else { DARK_BG };
    let (w, h) = (320u32, 240u32);
    let r = 30.0 + rng.unit() * 70.0;
    let cx = r + rng.unit() * (w as f32 - 2.0 * r);
    let cy = r + rng.unit() * (h as f32 - 2.0 * r);
    let mut img = draw(shape, rgb, base, w, h, cx, cy, r);
    for p in img.pixels_mut() {
        let n = (rng.below(21) as i32) - 10;
        for c in p.0.iter_mut() {
            *c = (*c as i32 + n).clamp(0, 255) as u8;
        }
    }
    img
}
