use std::io::{self, Read};
fn main() {
    let mut source = String::new();
    io::stdin().read_to_string(&mut source).unwrap();
    println!(
        "{}",
        serde_json::to_string(&atlas::analyze(&source)).unwrap()
    );
}
