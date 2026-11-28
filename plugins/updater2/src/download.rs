use std::path::Path;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use reqwest::{StatusCode, header};
use tokio::io::AsyncWriteExt;

use crate::Error;

/// Downloads `url` to `destination`, keeping the bytes received so far in a
/// sibling `.part` file so a later process can resume them with an HTTP range.
/// `destination` only appears once the complete file matches `signature`.
/// The caller must serialize downloads and cleanup for this destination.
pub async fn download_to_path(
    client: reqwest::ClientBuilder,
    url: &reqwest::Url,
    headers: &header::HeaderMap,
    signature: &str,
    pubkey: &str,
    destination: &Path,
    mut on_chunk: impl FnMut(u64, Option<u64>),
) -> Result<(), Error> {
    let partial = destination.with_extension("part");
    let identity_path = destination.with_extension("identity");
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // A version can be republished with different bytes or moved to another
    // URL. Truncate before recording the new identity so a process exit cannot
    // pair old bytes with a new signature.
    let identity = format!("{url}\n{signature}");
    if tokio::fs::read_to_string(&identity_path).await.ok() != Some(identity.clone()) {
        tokio::fs::File::create(&partial).await?;
        tokio::fs::write(&identity_path, &identity).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .await?;
    let mut offset = file.metadata().await?.len();

    let mut headers = headers.clone();
    headers
        .entry(header::ACCEPT)
        .or_insert(header::HeaderValue::from_static("application/octet-stream"));
    // Byte ranges must address the file itself, not a compressed encoding of it.
    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("identity"),
    );
    headers.remove(header::RANGE);
    headers.remove(header::IF_RANGE);
    let client = client
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .build()?;

    let mut reported = 0;
    let mut expected_total = None;
    loop {
        let mut request = client.get(url.clone()).headers(headers.clone());
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        let response = request.send().await?;
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
            // The app may have quit after the last write but before verification.
            if verify_file(&partial, signature, pubkey).await.is_ok() {
                on_chunk(offset.saturating_sub(reported), Some(offset));
                file.sync_all().await?;
                drop(file);
                tokio::fs::rename(&partial, destination).await?;
                let _ = tokio::fs::remove_file(&identity_path).await;
                return Ok(());
            }
            file.set_len(0).await?;
            offset = 0;
            expected_total = None;
            continue;
        }

        let (total, response_end) = match response.status() {
            StatusCode::OK => {
                // The server ignored the range: start over from its full body.
                file.set_len(0).await?;
                offset = 0;
                let total = response.content_length().or(expected_total);
                (total, total)
            }
            StatusCode::PARTIAL_CONTENT => {
                let (start, end, total) = response
                    .headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range)
                    .ok_or(Error::InvalidDownloadResponse)?;
                if start != offset
                    || start > end
                    || end >= total
                    || response
                        .content_length()
                        .is_some_and(|length| length != end - start + 1)
                {
                    return Err(Error::InvalidDownloadResponse);
                }
                (Some(total), Some(end + 1))
            }
            status => return Err(Error::DownloadStatus(status.as_u16())),
        };
        if expected_total
            .zip(total)
            .is_some_and(|(expected, total)| expected != total)
            || response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|encoding| encoding != "identity")
        {
            return Err(Error::InvalidDownloadResponse);
        }
        expected_total = total;

        // A full-response fallback can replay bytes already reported in this attempt.
        on_chunk(offset.saturating_sub(reported), total);
        reported = reported.max(offset);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let next_offset = offset
                .checked_add(chunk.len() as u64)
                .ok_or(Error::InvalidDownloadResponse)?;
            if response_end.is_some_and(|end| next_offset > end) {
                return Err(Error::InvalidDownloadResponse);
            }
            file.write_all(&chunk).await?;
            // Tokio writes can outlive write_all; report only bytes a new process would see.
            file.flush().await?;
            offset = next_offset;
            on_chunk(offset.saturating_sub(reported), total);
            reported = reported.max(offset);
        }
        if response_end.is_some_and(|end| end != offset) {
            return Err(Error::InvalidDownloadResponse);
        }
        // A server may cap a range response; ask for the rest.
        if total.is_none_or(|total| offset == total) {
            break;
        }
    }
    file.sync_all().await?;
    drop(file);
    if let Err(error) = verify_file(&partial, signature, pubkey).await {
        tokio::fs::remove_file(&partial).await?;
        return Err(error);
    }
    tokio::fs::rename(&partial, destination).await?;
    let _ = tokio::fs::remove_file(&identity_path).await;
    Ok(())
}

async fn verify_file(path: &Path, signature: &str, pubkey: &str) -> Result<(), Error> {
    let path = path.to_owned();
    let signature = signature.to_owned();
    let pubkey = pubkey.to_owned();
    tokio::task::spawn_blocking(move || {
        verify_signature(&std::fs::read(path)?, &signature, &pubkey)
    })
    .await
    .map_err(|e| Error::Io(std::io::Error::other(e)))?
}

// Mirrors tauri-plugin-updater: both the key and the signature are base64 of
// minisign's text format, and legacy (non-prehashed) signatures are accepted.
fn verify_signature(data: &[u8], signature: &str, pubkey: &str) -> Result<(), Error> {
    let decode = |value: &str| {
        STANDARD
            .decode(value)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(Error::InvalidSignature)
    };
    let public_key = minisign_verify::PublicKey::decode(&decode(pubkey)?)
        .map_err(|_| Error::InvalidSignature)?;
    let signature = minisign_verify::Signature::decode(&decode(signature)?)
        .map_err(|_| Error::InvalidSignature)?;
    public_key
        .verify(data, &signature, true)
        .map_err(|_| Error::InvalidSignature)
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}
