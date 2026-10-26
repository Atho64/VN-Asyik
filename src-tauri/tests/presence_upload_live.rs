//! Live checks for the presence image host.
//!
//! Ignored by default: these publish a small generated image to a **public** host
//! and need the network. Run them by hand when touching `presence::upload`:
//!
//! ```text
//! cargo test --test presence_upload_live -- --ignored --nocapture
//! ```
//!
//! Why they exist: the upload path is the one part of Rich Presence that cannot be
//! proven by a unit test with a seeded cache. A wrong field name, a missing
//! `User-Agent` rejected by the host's CDN, or a reply shape that is not the bare
//! URL all look identical to the app ("the card has no image"), and every one of
//! them is only visible against the real endpoint.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vn_core::presence::RpcImageHost;
use vn_tracker_lib::presence::upload::{upload_client, upload_to_host};

/// A small, valid JPEG. Generated rather than committed so the test carries no
/// fixture file, and encoded with the same crate the app uses to validate covers.
fn test_jpeg() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb([72, 148, 116]));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut std::io::Cursor::new(&mut bytes), 80)
        .encode_image(&image)
        .expect("encode a JPEG");
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG start of image");
    bytes
}

/// The real upload. Publishes one 8x8 image to catbox and prints its URL, so a
/// failure can be read rather than guessed at.
#[tokio::test]
#[ignore = "uploads one image to a public host; run manually"]
async fn catbox_accepts_a_small_image() {
    let url = upload_to_host(
        RpcImageHost::Catbox,
        None,
        test_jpeg(),
        "vn-asyik-upload-test.jpg",
    )
    .await
    .expect("catbox must accept an anonymous upload");
    println!("catbox url: {url}");
    assert!(
        url.starts_with("https://files.catbox.moe/"),
        "catbox answers with the bare hosted URL, got {url:?}"
    );
}

/// The request the app sends must identify itself.
///
/// Not politeness: both hosts sit behind Cloudflare, which refuses a client with no
/// `User-Agent`. The app sent none, so every upload was rejected and the card
/// silently lost its image. Checked on the wire against a loopback server, so the
/// assertion is about the bytes reqwest actually writes.
#[tokio::test]
async fn the_upload_request_identifies_itself() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buffer = vec![0_u8; 4096];
        let read = socket.read(&mut buffer).await.expect("read request");
        let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write response");
        request
    });

    let client = upload_client().expect("client");
    let _ = client.get(format!("http://{addr}/")).send().await;
    let request = server.await.expect("server task");

    assert!(
        request.contains("user-agent:"),
        "the presence upload must send a User-Agent; got:\n{request}"
    );
}
