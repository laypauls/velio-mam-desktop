//! 최소 CLI 파서 — `--key value` / `--flag`.

use std::collections::HashMap;
use std::path::PathBuf;

pub struct Args(HashMap<String, String>);

impl Args {
    pub fn parse() -> Self {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut map = HashMap::new();
        let mut i = 0;
        while i < raw.len() {
            let Some(key) = raw[i].strip_prefix("--") else {
                fail(&format!("unexpected argument: {}", raw[i]));
            };
            if i + 1 < raw.len() && !raw[i + 1].starts_with("--") {
                map.insert(key.to_string(), raw[i + 1].clone());
                i += 2;
            } else {
                map.insert(key.to_string(), String::new());
                i += 1;
            }
        }
        Self(map)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn num<T: std::str::FromStr>(&self, key: &str, default: T) -> T {
        match self.get(key) {
            None => default,
            Some(v) => v
                .parse()
                .unwrap_or_else(|_| fail(&format!("--{key}: invalid value '{v}'"))),
        }
    }

    pub fn required<T: std::str::FromStr>(&self, key: &str) -> T {
        match self.get(key) {
            None => fail(&format!("--{key} is required")),
            Some(v) => v
                .parse()
                .unwrap_or_else(|_| fail(&format!("--{key}: invalid value '{v}'"))),
        }
    }

    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.get(key).map(PathBuf::from)
    }
}

pub fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}
