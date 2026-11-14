//! The client against a running sync server, reading the sign-in code from its Mailpit inbox.
//! Ignored by default; run with
//!   EDGE_TEST_SERVER=https://edge-sync.forcesync.in EDGE_TEST_MAILPIT=http://100.73.176.114:58025 \
//!   cargo test -p notes-core --features client --test client_live -- --ignored
#![cfg(feature = "client")]

use std::sync::Mutex;

use notes_core::client::{Client, ClientError};
use notes_core::Store;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"))
}

fn code_for(mailpit: &str, email: &str) -> String {
    let url = format!("{mailpit}/api/v1/search?query=to:{email}");
    let v: serde_json::Value = ureq::get(&url).call().unwrap().body_mut().read_json().unwrap();
    v["messages"][0]["Subject"].as_str().unwrap()[..6].to_owned()
}

#[test]
#[ignore]
fn two_devices_sync_through_the_real_server() {
    let (server, mailpit) = (env("EDGE_TEST_SERVER"), env("EDGE_TEST_MAILPIT"));
    let client = Client::new(&server);
    let email = format!("client-{}@edge.test", notes_core::Uuid::now_v7().simple());

    client.start_sign_in(&email).unwrap();
    assert_eq!(client.start_sign_in(&email), Err(ClientError::SlowDown));
    assert_eq!(client.verify(&email, "abcdef", "phone"), Err(ClientError::WrongCode));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let phone_token = client.verify(&email, &code_for(&mailpit, &email), "phone").unwrap();
    assert!(client.me(&phone_token).unwrap().active);

    let phone = Mutex::new(Store::open_in_memory("phone").unwrap());
    let laptop = Mutex::new(Store::open_in_memory("laptop").unwrap());
    phone.lock().unwrap().create("Hello from the phone").unwrap();
    assert_eq!(client.sync(&phone, &phone_token).unwrap(), 0);
    // Same account on the laptop. The code table allows the next code after a minute, so the
    // laptop reuses the phone's session here; the server tests cover separate sessions.
    assert_eq!(client.sync(&laptop, &phone_token).unwrap(), 1);
    assert_eq!(laptop.lock().unwrap().list().unwrap()[0].body, "Hello from the phone");

    client.sign_out(&phone_token).unwrap();
    assert_eq!(client.sync(&phone, &phone_token), Err(ClientError::SignedOut));
}
