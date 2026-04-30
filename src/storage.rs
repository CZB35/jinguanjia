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
    let upload_root = resolve_upload_root(&state.config.storage.upload_dir);
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
    let mime_type = content_type.map(ToString::to_string).unwrap_or_else(|| {
        mime_guess::from_path(original_name)
            .first_or_octet_stream()
            .to_string()
    });

    Ok(StoredFile {
        original_name: original_name.to_string(),
        relative_path,
        mime_type,
        file_size: bytes.len() as i64,
    })
}

pub fn full_path(state: &AppState, relative_path: &str) -> PathBuf {
    resolve_upload_root(&state.config.storage.upload_dir).join(relative_path)
}

pub fn resolve_upload_root(upload_dir: &str) -> PathBuf {
    let path = PathBuf::from(upload_dir);
    if path.is_absolute() {
        return path;
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        if exe_dir
            .file_name()
            .and_then(|item| item.to_str())
            .map(|name| name.eq_ignore_ascii_case("release") || name.eq_ignore_ascii_case("debug"))
            .unwrap_or(false)
            && exe_dir
                .parent()
                .and_then(|item| item.file_name())
                .and_then(|item| item.to_str())
                .map(|name| name.eq_ignore_ascii_case("target"))
                .unwrap_or(false)
            && let Some(project_root) = exe_dir.parent().and_then(|item| item.parent())
        {
            return project_root.join(upload_dir);
        }

        return exe_dir.join(upload_dir);
    }

    path
}

pub fn public_url(relative_path: &str) -> String {
    let normalized = relative_path
        .trim()
        .trim_start_matches('/')
        .replace('\\', "/");
    format!("/uploads/{}", normalized)
}
