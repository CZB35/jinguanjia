use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct StoredFile {
    pub original_name: String,
    pub relative_path: String,
    pub mime_type: String,
    pub file_size: i64,
}

pub async fn save_bytes(
    state: &AppState,
    sub_dir: &str,
    original_name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<StoredFile> {
    let upload_root = PathBuf::from(&state.config.storage.upload_dir);
    let dir = upload_root.join(sub_dir);
    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("failed to create upload dir: {}", dir.display()))?;

    let ext = Path::new(original_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("bin");
    let filename = format!("{}.{}", Uuid::new_v4(), ext);
    let absolute_path = dir.join(&filename);
    fs::write(&absolute_path, bytes)
        .await
        .with_context(|| format!("failed to write upload file: {}", absolute_path.display()))?;

    let relative_path = format!("{}/{}", sub_dir.replace('\\', "/"), filename);
    let mime_type = content_type
        .map(ToString::to_string)
        .unwrap_or_else(|| mime_guess::from_path(original_name).first_or_octet_stream().to_string());

    Ok(StoredFile {
        original_name: original_name.to_string(),
        relative_path,
        mime_type,
        file_size: bytes.len() as i64,
    })
}

pub fn full_path(state: &AppState, relative_path: &str) -> PathBuf {
    PathBuf::from(&state.config.storage.upload_dir).join(relative_path)
}

pub fn public_url(state: &AppState, relative_path: &str) -> String {
    let base = state.config.storage.public_base_url.trim_end_matches('/');
    let upload_dir = state.config.storage.upload_dir.trim_matches('/').replace('\\', "/");
    let relative = relative_path.trim_start_matches('/');
    format!("{}/{}/{}", base, upload_dir, relative)
}
