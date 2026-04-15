//! File download utilities
//!
//! Handles downloading files with progress reporting, including special
//! handling for Google Drive URLs with virus scan confirmation pages.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::progress::InstallProgress;
use crate::push_to::{InstallStage, PushToError, PushToResult};

/// Download a file with progress reporting
///
/// Handles both regular URLs and Google Drive URLs (with virus scan confirmation).
pub async fn download_file(
    url: &str,
    dest: &Path,
    component: &str,
    stage: Option<InstallStage>,
    tx: mpsc::Sender<InstallProgress>,
) -> PushToResult<()> {
    info!(
        url = %url,
        dest = %dest.display(),
        component = %component,
        "Starting download"
    );

    let client = Arc::new(create_http_client()?);

    let is_google_drive = url.contains("drive.google.com");
    let download_url = prepare_download_url(url, is_google_drive);

    info!(url = %download_url, is_google_drive = is_google_drive, "Sending HTTP GET request...");

    let response = client.get(&download_url).send().await.map_err(|e| {
        error!(error = %e, url = %url, "Download request failed");
        PushToError::InstallFailed(format!("Download request failed: {}", e))
    })?;

    let status = response.status();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    info!(
        status = %status,
        final_url = %final_url,
        content_type = %content_type,
        "Received HTTP response"
    );

    if !status.is_success() {
        error!(status = %status, "Download failed with non-success status");
        return Err(PushToError::InstallFailed(format!(
            "Download failed with status: {}",
            status
        )));
    }

    // Handle Google Drive virus scan warning page
    let response = if is_google_drive && content_type.contains("text/html") {
        handle_google_drive_confirmation(response, url, &client).await?
    } else {
        response
    };

    // Re-check status after potential redirect
    let status = response.status();
    if !status.is_success() {
        error!(status = %status, "Download failed after confirmation");
        return Err(PushToError::InstallFailed(format!(
            "Download failed with status: {}",
            status
        )));
    }

    download_with_progress(response, dest, component, stage, tx).await
}

fn create_http_client() -> PushToResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(3600)) // 1 hour timeout for large files
        .cookie_store(true) // Required for Google Drive confirmation
        .build()
        .map_err(|e| {
            error!(error = %e, "Failed to create HTTP client");
            PushToError::InstallFailed(format!("Failed to create HTTP client: {}", e))
        })
}

fn prepare_download_url(url: &str, is_google_drive: bool) -> String {
    if is_google_drive {
        // Add confirm=1 to bypass the "file is too large for virus scan" warning
        if url.contains('?') {
            format!("{}&confirm=1", url)
        } else {
            format!("{}?confirm=1", url)
        }
    } else {
        url.to_string()
    }
}

async fn handle_google_drive_confirmation(
    response: reqwest::Response,
    original_url: &str,
    client: &reqwest::Client,
) -> PushToResult<reqwest::Response> {
    warn!("Google Drive returned HTML page, attempting to extract download link");

    let html = response.text().await.map_err(|e| {
        error!(error = %e, "Failed to read HTML response");
        PushToError::InstallFailed(format!("Failed to read HTML response: {}", e))
    })?;

    let confirm_url = extract_google_drive_confirm_url(&html, original_url).ok_or_else(|| {
        error!("Could not find download confirmation link in Google Drive HTML");
        debug!(html_preview = %html.chars().take(500).collect::<String>(), "HTML preview");
        PushToError::InstallFailed(
            "Google Drive virus scan page: could not extract download link. \
             The file may be unavailable or the page format changed."
                .to_string(),
        )
    })?;

    info!(confirm_url = %confirm_url, "Found Google Drive confirmation URL, retrying download");

    client.get(&confirm_url).send().await.map_err(|e| {
        error!(error = %e, "Confirmation download request failed");
        PushToError::InstallFailed(format!("Confirmation download failed: {}", e))
    })
}

async fn download_with_progress(
    response: reqwest::Response,
    dest: &Path,
    component: &str,
    stage: Option<InstallStage>,
    tx: mpsc::Sender<InstallProgress>,
) -> PushToResult<()> {
    let total_size = response.content_length();
    info!(
        content_length = ?total_size,
        component = %component,
        "Starting download stream"
    );

    let mut downloaded: u64 = 0;
    let mut last_log_time = std::time::Instant::now();
    let mut last_log_bytes: u64 = 0;

    let mut file = File::create(dest).map_err(|e| {
        error!(error = %e, path = %dest.display(), "Failed to create file");
        PushToError::InstallFailed(format!("Failed to create file: {}", e))
    })?;

    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            error!(
                error = %e,
                bytes_downloaded = downloaded,
                "Download stream error"
            );
            PushToError::InstallFailed(format!("Download stream error: {}", e))
        })?;

        file.write_all(&chunk).map_err(|e| {
            error!(error = %e, "Failed to write chunk to file");
            PushToError::InstallFailed(format!("Failed to write file: {}", e))
        })?;

        downloaded += chunk.len() as u64;

        // Log progress every 5 seconds with speed calculation
        log_download_progress(
            component,
            downloaded,
            total_size,
            &mut last_log_time,
            &mut last_log_bytes,
        );

        // Send WebSocket progress update every ~1MB
        if downloaded % (1024 * 1024) < chunk.len() as u64 {
            let _ = tx
                .send(InstallProgress::Downloading {
                    component: component.to_string(),
                    bytes_downloaded: downloaded,
                    total_bytes: total_size,
                    stage,
                })
                .await;
        }
    }

    // Final progress update
    let _ = tx
        .send(InstallProgress::Downloading {
            component: component.to_string(),
            bytes_downloaded: downloaded,
            total_bytes: total_size,
            stage,
        })
        .await;

    info!(
        component = %component,
        total_bytes = downloaded,
        total_mb = format!("{:.1}", downloaded as f64 / 1024.0 / 1024.0),
        "Download completed"
    );
    Ok(())
}

fn log_download_progress(
    component: &str,
    downloaded: u64,
    total_size: Option<u64>,
    last_log_time: &mut std::time::Instant,
    last_log_bytes: &mut u64,
) {
    let now = std::time::Instant::now();
    let elapsed = now.duration_since(*last_log_time);

    if elapsed.as_secs() >= 5 {
        let bytes_since_last = downloaded - *last_log_bytes;
        let speed_kbps = (bytes_since_last as f64 / elapsed.as_secs_f64()) / 1024.0;
        let percent = total_size.map(|t| (downloaded as f64 / t as f64) * 100.0);

        info!(
            component = %component,
            downloaded_mb = format!("{:.1}", downloaded as f64 / 1024.0 / 1024.0),
            total_mb = total_size.map(|t| format!("{:.1}", t as f64 / 1024.0 / 1024.0)),
            percent = percent.map(|p| format!("{:.1}%", p)),
            speed_kbps = format!("{:.1}", speed_kbps),
            "Download progress"
        );

        *last_log_time = now;
        *last_log_bytes = downloaded;
    }
}

/// Extract the actual download URL from Google Drive's virus scan warning HTML page.
fn extract_google_drive_confirm_url(html: &str, original_url: &str) -> Option<String> {
    // Method 1: Look for the form action with confirmation token
    if let Some(form_action) = extract_form_download_url(html) {
        return Some(form_action);
    }

    // Method 2: Look for href with /uc?export=download&confirm=
    if let Some(href_url) = extract_href_download_url(html) {
        return Some(href_url);
    }

    // Method 3: Look for download link with confirm parameter in any format
    extract_direct_confirm_url(html, original_url)
}

fn extract_form_download_url(html: &str) -> Option<String> {
    let action_pattern = r#"action="([^"]+)""#;
    let action_regex = regex::Regex::new(action_pattern).ok()?;

    let mut action_url = None;
    for cap in action_regex.captures_iter(html) {
        let url = cap.get(1)?.as_str();
        if url.contains("download") || url.contains("usercontent.google") {
            action_url = Some(url.to_string());
            break;
        }
    }

    let action_url = action_url?;

    // Extract hidden input values
    let input_pattern = r#"<input[^>]+name="([^"]+)"[^>]+value="([^"]*)""#;
    let input_regex = regex::Regex::new(input_pattern).ok()?;

    let mut params: Vec<(String, String)> = Vec::new();

    for cap in input_regex.captures_iter(html) {
        let name = cap.get(1)?.as_str().to_string();
        let value = cap.get(2)?.as_str().to_string();
        params.push((name, value));
    }

    // Also try alternate pattern where value comes before name
    let input_pattern2 = r#"<input[^>]+value="([^"]*)"[^>]+name="([^"]+)""#;
    if let Ok(input_regex2) = regex::Regex::new(input_pattern2) {
        for cap in input_regex2.captures_iter(html) {
            let value = cap.get(1)?.as_str().to_string();
            let name = cap.get(2)?.as_str().to_string();
            if !params.iter().any(|(n, _)| n == &name) {
                params.push((name, value));
            }
        }
    }

    if params.is_empty() {
        return None;
    }

    let query_string: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let full_url = if action_url.contains('?') {
        format!("{}&{}", action_url, query_string)
    } else {
        format!("{}?{}", action_url, query_string)
    };

    info!(extracted_url = %full_url, "Extracted form download URL");
    Some(full_url)
}

fn extract_href_download_url(html: &str) -> Option<String> {
    let patterns = [
        r#"href="(/uc\?[^"]+confirm[^"]+)""#,
        r#"href="(https://[^"]*drive[^"]*confirm[^"]+)""#,
    ];

    for pattern in patterns {
        if let Ok(regex) = regex::Regex::new(pattern) {
            if let Some(cap) = regex.captures(html) {
                if let Some(url_match) = cap.get(1) {
                    let mut url = url_match.as_str().to_string();
                    url = url.replace("&amp;", "&");

                    if url.starts_with('/') {
                        url = format!("https://drive.google.com{}", url);
                    }

                    info!(extracted_url = %url, "Extracted href download URL");
                    return Some(url);
                }
            }
        }
    }

    None
}

fn extract_direct_confirm_url(html: &str, original_url: &str) -> Option<String> {
    let confirm_patterns = [
        r#"confirm=([a-zA-Z0-9_-]+)"#,
        r#"&amp;confirm=([a-zA-Z0-9_-]+)"#,
    ];

    let mut confirm_token = None;
    for pattern in confirm_patterns {
        if let Ok(regex) = regex::Regex::new(pattern) {
            if let Some(cap) = regex.captures(html) {
                if let Some(token) = cap.get(1) {
                    confirm_token = Some(token.as_str().to_string());
                    break;
                }
            }
        }
    }

    let confirm_token = confirm_token?;

    let id_regex = regex::Regex::new(r"id=([a-zA-Z0-9_-]+)").ok()?;
    let id = id_regex
        .captures(original_url)?
        .get(1)?
        .as_str()
        .to_string();

    let url = format!(
        "https://drive.usercontent.google.com/download?id={}&export=download&confirm={}",
        id, confirm_token
    );

    info!(extracted_url = %url, confirm_token = %confirm_token, "Built confirm URL from token");
    Some(url)
}
