// embed_migrations! 는 마이그레이션 파일의 추가·삭제를 추적하지 않는다 — 폴더 변경 시 다시 컴파일한다 (P1-7 결함 2).
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
