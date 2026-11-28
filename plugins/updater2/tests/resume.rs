use std::{path::Path, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use tauri_plugin_updater2::download::download_to_path;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

// Public test vector from minisign-verify, signing the four bytes "test".
const PUBKEY: &str = "untrusted comment: minisign public key
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
const SIGNATURE: &str = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

async fn download(
    url: &str,
    destination: &Path,
    on_chunk: impl FnMut(u64, Option<u64>),
) -> Result<(), tauri_plugin_updater2::Error> {
    download_to_path(
        reqwest::ClientBuilder::new()
            .no_proxy()
            .timeout(Duration::from_secs(5)),
        &url.parse().unwrap(),
        &Default::default(),
        &STANDARD.encode(SIGNATURE),
        &STANDARD.encode(PUBKEY),
        destination,
        on_chunk,
    )
    .await
}

fn identity(url: &str) -> String {
    format!("{url}\n{}", STANDARD.encode(SIGNATURE))
}

async fn request(listener: &tokio::net::TcpListener) -> (tokio::net::TcpStream, String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(socket.read_u8().await.unwrap());
    }
    (socket, String::from_utf8(bytes).unwrap().to_lowercase())
}

#[tokio::test]
async fn interrupted_download_resumes_saved_bytes_after_process_exit() {
    if let Ok(url) = std::env::var("LIBRELOG_RESUME_TEST_URL") {
        let path = std::env::var("LIBRELOG_RESUME_TEST_PATH").unwrap();
        download(&url, Path::new(&path), |bytes, _| {
            if bytes > 0 {
                println!("partial saved");
            }
        })
        .await
        .unwrap();
        return;
    }
    for final_response in [
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 3-3/4\r\n\r\nt",
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("1.0.1.bin");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/update", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut first, headers) = request(&listener).await;
            assert!(!headers.contains("range:"));
            first
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nte")
                .await
                .unwrap();

            let (mut second, headers) = request(&listener).await;
            assert!(headers.contains("range: bytes=2-\r\n"));
            second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 2-2/4\r\n\r\ns").await.unwrap();
            drop(second);

            let (mut third, headers) = request(&listener).await;
            assert!(headers.contains("range: bytes=3-\r\n"));
            third.write_all(final_response.as_bytes()).await.unwrap();
        });
        let mut process = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_download_resumes_saved_bytes_after_process_exit",
                "--nocapture",
            ])
            .env("LIBRELOG_RESUME_TEST_URL", &url)
            .env("LIBRELOG_RESUME_TEST_PATH", &destination)
            .env("RUST_TEST_THREADS", "1")
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut output = BufReader::new(process.stdout.take().unwrap()).lines();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !output
                .next_line()
                .await
                .unwrap()
                .expect("download process exited")
                .ends_with("partial saved")
            {}
        })
        .await
        .unwrap();
        process.kill().await.unwrap();
        process.wait().await.unwrap();
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(destination.with_extension("part")).unwrap(),
            b"te"
        );

        let mut progress = Vec::new();
        download(&url, &destination, |bytes, total| {
            progress.push((bytes, total))
        })
        .await
        .unwrap();
        assert_eq!(progress.first(), Some(&(2, Some(4))));
        assert_eq!(progress.iter().map(|(bytes, _)| bytes).sum::<u64>(), 4);
        assert_eq!(std::fs::read(&destination).unwrap(), b"test");
        assert!(!destination.with_extension("part").exists());
        assert!(!destination.with_extension("identity").exists());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn resumed_download_never_publishes_unverified_or_misaligned_bytes() {
    for (response, saved, identity_matches, succeeds) in [
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest",
            "te",
            true,
            true,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest",
            "old",
            false,
            true,
        ),
        (
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n",
            "test",
            true,
            true,
        ),
        (
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n",
            "oops",
            true,
            true,
        ),
        (
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nContent-Range: bytes 0-1/4\r\n\r\nst",
            "te",
            true,
            false,
        ),
        (
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nContent-Range: bytes 2-3/4\r\n\r\nxx",
            "te",
            true,
            false,
        ),
        (
            "HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 2-2/4\r\n\r\n2\r\nst\r\n0\r\n\r\n",
            "te",
            true,
            false,
        ),
        (
            "HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 2-3/4\r\n\r\n1\r\ns\r\n0\r\n\r\n",
            "te",
            true,
            false,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("1.0.1.bin");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/update", listener.local_addr().unwrap());
        std::fs::write(destination.with_extension("part"), saved).unwrap();
        std::fs::write(
            destination.with_extension("identity"),
            if identity_matches {
                identity(&url)
            } else {
                format!("{url}\nold signature")
            },
        )
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, headers) = request(&listener).await;
            assert_eq!(headers.contains("range:"), identity_matches);
            socket.write_all(response.as_bytes()).await.unwrap();
            drop(socket);
            if response.starts_with("HTTP/1.1 416") && saved == "oops" {
                let (mut retry, headers) = request(&listener).await;
                assert!(!headers.contains("range:"));
                retry
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest")
                    .await
                    .unwrap();
            }
        });
        let result = download(&url, &destination, |_, _| {}).await;
        assert_eq!(result.is_ok(), succeeds, "{response}: {result:?}");
        assert_eq!(destination.exists(), succeeds);
        if succeeds {
            assert_eq!(std::fs::read(&destination).unwrap(), b"test");
        }
        server.await.unwrap();
    }
}
