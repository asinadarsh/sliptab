//! Tiny CLI over the store, for poking at a real database file.
//!   cargo run --example notes -- add "buy milk"
//!   cargo run --example notes -- list
//!   cargo run --example notes -- search milk
//! Database path: $EDGE_DB, default ./notes.db

use notes_core::Store;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = std::env::var("EDGE_DB").unwrap_or_else(|_| "notes.db".into());
    let mut store = Store::open(db, "cli")?;
    let notes = match args.first().map(String::as_str) {
        Some("add") => vec![store.create(&args[1..].join(" "))?],
        Some("list") => store.list()?,
        Some("search") => store.search(&args[1..].join(" "))?,
        _ => {
            eprintln!("usage: notes add <text> | list | search <query>");
            std::process::exit(2);
        }
    };
    for n in notes {
        println!(
            "{}{}  v{}  {}",
            if n.pinned { "📌 " } else { "" },
            n.id,
            n.version,
            n.title
        );
    }
    Ok(())
}
