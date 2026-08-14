//! Exercises the real system clipboard. Ignored by default because it needs a live
//! desktop session and mutates the user's clipboard.
//!
//!     cargo test -p gramit-input --test clipboard_live -- --ignored --nocapture

use gramit_input::clipboard::{self, ArboardClipboard, Clipboard, ClipboardSnapshot};

#[tokio::test]
#[ignore = "needs a desktop session and touches the real clipboard"]
async fn round_trips_text_through_the_system_clipboard() {
    let clipboard = ArboardClipboard::new().expect("open the system clipboard");

    let original = clipboard::snapshot(&clipboard).await.expect("snapshot");
    println!("clipboard before: {original:?}");

    let sample = "gramit clipboard check — héllo wörld\nsecond line";
    clipboard.set_text(sample.to_string()).await.expect("set text");

    let read_back = clipboard.get_text().await.expect("get text");
    assert_eq!(read_back.as_deref(), Some(sample), "text must survive the round trip");

    clipboard::restore(&clipboard, &original).await.expect("restore");
    let after = clipboard::snapshot(&clipboard).await.expect("snapshot after restore");
    assert_eq!(after, original, "the user's clipboard must be put back exactly");

    println!("clipboard after restore: {after:?}");
}

#[tokio::test]
#[ignore = "needs a desktop session and touches the real clipboard"]
async fn clear_empties_the_clipboard() {
    let clipboard = ArboardClipboard::new().expect("open the system clipboard");
    let original = clipboard::snapshot(&clipboard).await.expect("snapshot");

    clipboard.set_text("gramit clear check".into()).await.expect("set text");
    clipboard.clear().await.expect("clear");

    assert_eq!(
        clipboard::snapshot(&clipboard).await.expect("snapshot"),
        ClipboardSnapshot::Empty,
        "a cleared clipboard must read as empty"
    );

    clipboard::restore(&clipboard, &original).await.expect("restore");
}
