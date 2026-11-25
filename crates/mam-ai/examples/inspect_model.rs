//! ONNX 모델의 입출력 이름·형태를 출력한다: `inspect_model <model.onnx>`

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: inspect_model <model.onnx>");
    let env = ort::init().build().expect("env");
    let session = ort::session::Session::builder(&env)
        .unwrap()
        .commit_from_file(&path)
        .expect("load");
    for i in session.inputs() {
        println!("input  {} {:?}", i.name(), i.dtype());
    }
    for o in session.outputs() {
        println!("output {} {:?}", o.name(), o.dtype());
    }
}
