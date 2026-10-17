//! Print a JSON project report for every `.astro` file under the given directory.
use std::{fs, path::Path};

fn collect(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "astro") {
            let source = fs::read_to_string(&path).unwrap();
            out.push((path.display().to_string(), source));
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("usage: atlas-report <dir>");
    let mut sources = Vec::new();
    collect(Path::new(&root), &mut sources);
    let report = atlas::analyze_project(sources).report();
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
