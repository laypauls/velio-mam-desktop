//! 합성 데이터 어휘. 정답을 부분 문자열로 계산하므로 **텍스트 어휘끼리·텍스트 어휘와 파일명 어휘가
//! 서로의 부분 문자열이 되지 않게** 골랐다 (`self_check` 가 검증한다). 앞쪽 항목일수록 자주 쓰인다.

/// 태그·AI 텍스트용 한국어 명사.
pub const TEXT_KO: &[&str] = &[
    // 동물
    "강아지",
    "고양이",
    "토끼",
    "거북이",
    "사슴",
    "다람쥐",
    "앵무새",
    "펭귄",
    "코끼리",
    "호랑이",
    "기린",
    "원숭이",
    "햄스터",
    "오리",
    "갈매기",
    "독수리",
    "나비",
    "잠자리",
    // 자연
    "바다",
    "하늘",
    "구름",
    "노을",
    "해변",
    "파도",
    "폭포",
    "호수",
    "계곡",
    "단풍",
    "벚꽃",
    "눈꽃",
    "일출",
    "별빛",
    "달빛",
    "안개",
    "무지개",
    "갯벌",
    "사막",
    "초원",
    "강물",
    // 음식
    "김치",
    "비빔밥",
    "라면",
    "햄버거",
    "피자",
    "커피",
    "케이크",
    "초밥",
    "삼겹살",
    "떡볶이",
    "샐러드",
    "아이스크림",
    "과일",
    "불고기",
    "냉면",
    "파스타",
    "샌드위치",
    "와플",
    "도시락",
    // 장소
    "공원",
    "학교",
    "도서관",
    "카페",
    "식당",
    "시장",
    "공항",
    "기차역",
    "놀이터",
    "박물관",
    "미술관",
    "운동장",
    "경기장",
    "해수욕장",
    "사무실",
    "거실",
    "부엌",
    "정원",
    "옥상",
    "지하철",
    // 행사·활동
    "결혼식",
    "생일",
    "졸업식",
    "입학식",
    "운동회",
    "소풍",
    "산책",
    "등산",
    "캠핑",
    "축제",
    "공연",
    "콘서트",
    "파티",
    "낚시",
    "수영",
    "달리기",
    "자전거",
    "마라톤",
    "스키",
    "요가",
];

/// 고유명사 태그 (ko-dic 이 쪼갤 수 있는 후보 — 품질 평가에서 관찰한다).
pub const TEXT_PROPER: &[&str] = &[
    "제주도",
    "한라산",
    "경복궁",
    "에버랜드",
    "롯데월드",
    "남산타워",
    "해운대",
    "광안대교",
];

/// 태그·AI 키워드용 영어 단어.
pub const TEXT_EN: &[&str] = &[
    "sunset",
    "mountain",
    "family",
    "birthday",
    "wedding",
    "portrait",
    "landscape",
    "concert",
    "festival",
    "football",
    "bicycle",
    "kitchen",
    "airplane",
    "bridge",
    "river",
    "forest",
    "snow",
    "flower",
    "coffee",
    "dinner",
    "garden",
    "window",
    "lake",
    "museum",
];

/// 파일명 한국어 지명 (2음절 고유명사).
pub const FN_PLACES: &[&str] = &[
    "부산", "제주", "서울", "강릉", "경주", "전주", "속초", "여수", "대구", "인천", "광주", "대전",
    "울산", "춘천", "순천", "안동", "목포", "포항",
];

/// 파일명 한국어 단어 (복합명사 포함: `여행사진` 은 ko-dic 에서 한 토큰이 되는 사례).
pub const FN_KO: &[&str] = &[
    "가족",
    "친구",
    "동료",
    "아기",
    "엄마",
    "아빠",
    "할머니",
    "회사",
    "출장",
    "여행",
    "휴가",
    "사진",
    "영상",
    "녹음",
    "회의록",
    "프로젝트",
    "계약서",
    "견적",
    "보고",
    "기념",
    "모임",
    "동창회",
    "행사",
    "워크숍",
    "인터뷰",
    "강의",
    "수업",
    "발표자료",
    "시안",
    "최종",
    "수정",
    "원본",
    "백업",
    // 복합명사
    "졸업사진",
    "가족사진",
    "여행사진",
    "증명사진",
    "결혼사진",
    "여행영상",
    "가족영상",
];

/// 파일명 영어 단어.
pub const FN_EN: &[&str] = &[
    "trip",
    "vacation",
    "beach",
    "photo",
    "video",
    "project",
    "report",
    "notes",
    "final",
    "draft",
    "backup",
    "holiday",
    "album",
    "edit",
    "selfie",
    "hiking",
    "travel",
    "memo",
    "scan",
    "design",
    "logo",
    "banner",
    "summer",
    "meeting",
    "interview",
    "lecture",
    "release",
    "sample",
    "test",
];

/// 파일명 접두어 (카메라·스크린샷·녹음 등).
pub const FN_PREFIXES: &[&str] = &[
    "img",
    "dsc",
    "vid",
    "mvi",
    "screenshot",
    "스크린샷",
    "recording",
    "녹음",
    "voice",
    "note",
];

/// 자산 설명 템플릿의 고정 조각 (어휘와 겹치면 안 된다).
pub const FRAGMENTS: &[&str] = &[
    "바라보고",
    "있다",
    "함께",
    "시간을",
    "보내는",
    "모습",
    "배경으로",
    "서",
    "잘",
    "어울린다",
    "찾아",
    "걸어간다",
    "메모",
    "다시",
    "확인할",
    "것",
];

/// 조사 종류. 받침 유무로 형태가 정해진다.
#[derive(Clone, Copy)]
pub enum Josa {
    Subject,
    Object,
    With,
    Topic,
    Toward,
}

/// 마지막 글자의 받침 여부 (한글 음절이 아니면 없음으로 본다).
pub fn has_batchim(word: &str) -> bool {
    word.chars()
        .last()
        .map(|c| {
            let u = c as u32;
            (0xAC00..=0xD7A3).contains(&u) && !(u - 0xAC00).is_multiple_of(28)
        })
        .unwrap_or(false)
}

fn rieul_batchim(word: &str) -> bool {
    word.chars()
        .last()
        .map(|c| (c as u32).wrapping_sub(0xAC00) % 28 == 8)
        .unwrap_or(false)
}

/// 단어에 맞는 조사를 붙인 문자열.
pub fn with_josa(word: &str, josa: Josa) -> String {
    let b = has_batchim(word);
    let p = match josa {
        Josa::Subject => {
            if b {
                "이"
            } else {
                "가"
            }
        }
        Josa::Object => {
            if b {
                "을"
            } else {
                "를"
            }
        }
        Josa::With => {
            if b {
                "과"
            } else {
                "와"
            }
        }
        Josa::Topic => {
            if b {
                "은"
            } else {
                "는"
            }
        }
        Josa::Toward => {
            if b && !rieul_batchim(word) {
                "으로"
            } else {
                "로"
            }
        }
    };
    format!("{word}{p}")
}

/// 검사에 쓰는 조사 문자열 전체 (질의 쪽 변형 포함).
pub const ALL_PARTICLES: &[&str] = &[
    "이", "가", "을", "를", "와", "과", "은", "는", "의", "에서", "에", "으로", "로", "도", "만",
];

/// 모든 텍스트 어휘 (한국어 + 고유명사 + 영어).
pub fn text_lemmas() -> Vec<&'static str> {
    TEXT_KO
        .iter()
        .chain(TEXT_PROPER)
        .chain(TEXT_EN)
        .copied()
        .collect()
}

/// 어휘 충돌 검사 — 위반하면 이유를 담은 목록을 돌려준다.
pub fn self_check() -> Vec<String> {
    let lemmas = text_lemmas();
    let mut bad = Vec::new();
    let mut fn_words: Vec<&str> = Vec::new();
    for list in [FN_PLACES, FN_KO, FN_EN, FN_PREFIXES, FRAGMENTS] {
        fn_words.extend_from_slice(list);
    }
    for (i, l) in lemmas.iter().enumerate() {
        for w in &fn_words {
            if w.to_lowercase().contains(l) {
                bad.push(format!(
                    "text lemma '{l}' is inside filename/fragment word '{w}'"
                ));
            }
        }
        for (j, m) in lemmas.iter().enumerate() {
            if i == j {
                continue;
            }
            if m.contains(l) {
                bad.push(format!("text lemma '{l}' is inside text lemma '{m}'"));
            }
            for p in ALL_PARTICLES {
                if format!("{m}{p}").contains(l) {
                    bad.push(format!("text lemma '{l}' appears in '{m}{p}'"));
                }
            }
        }
    }
    bad
}
