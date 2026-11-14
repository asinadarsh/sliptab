//! Live check of the details fetch: `cargo run -p notes-core --features client --example preview -- <url>...`
fn main() {
    for arg in std::env::args().skip(1) {
        let Some(link) = notes_core::Link::from_text(&arg) else { eprintln!("not a link: {arg}"); continue };
        let t = std::time::Instant::now();
        let r = notes_core::client::fetch_preview(&link);
        println!("{} [{:.1}s] {}\n  {r:?}\n", link.platform, t.elapsed().as_secs_f32(), link.url);
    }
}
