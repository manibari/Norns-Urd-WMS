//! Send one photo through the same recognition call the app makes.
//!
//!     GEMINI_API_KEY=... cargo run -p urdwms-core --example recognize -- photo.jpg [model]
//!     GEMINI_API_KEY=... cargo run -p urdwms-core --example recognize -- --check [model]

use urdwms_core::recognition::{DEFAULT_MODEL, GeminiProvider, RecognitionProvider};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("photo path, or --check");
    let model = args.next().unwrap_or_else(|| DEFAULT_MODEL.into());
    let key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY");
    if path == "--check" {
        println!("{:?}", GeminiProvider::new(key, model).check());
        return;
    }
    let bytes = std::fs::read(&path).expect("read photo");
    let started = std::time::Instant::now();
    let reading = GeminiProvider::new(key, model).recognize(&bytes);
    println!("{:.1}s {reading:#?}", started.elapsed().as_secs_f64());
}
