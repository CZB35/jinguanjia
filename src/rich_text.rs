use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::{state::AppState, storage};

const DATA_IMAGE_PREFIX: &str = "data:image/";
const MAX_INLINE_IMAGE_BYTES: usize = 10 * 1024 * 1024;

pub async fn persist_inline_images(state: &AppState, html: &str, sub_dir: &str) -> Result<String> {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0;

    while let Some((_, quote, data_start)) = find_next_inline_image_src(html, cursor) {
        output.push_str(&html[cursor..data_start]);
        let content_start = data_start + DATA_IMAGE_PREFIX.len();
        let Some(close_rel) = html[content_start..].find(quote) else {
            output.push_str(&html[data_start..]);
            return Ok(output);
        };
        let close = content_start + close_rel;
        let data_uri = &html[data_start..close];
        let stored_url = persist_data_image(state, data_uri, sub_dir).await?;
        output.push_str(&stored_url);
        cursor = close;
    }

    output.push_str(&html[cursor..]);
    Ok(output)
}

async fn persist_data_image(state: &AppState, data_uri: &str, sub_dir: &str) -> Result<String> {
    let (mime_type, encoded) = data_uri
        .split_once(";base64,")
        .ok_or_else(|| anyhow!("invalid inline image data uri"))?;
    if !mime_type.starts_with(DATA_IMAGE_PREFIX) {
        return Err(anyhow!("inline image is not an image data uri"));
    }

    let ext = match mime_type {
        "data:image/jpeg" | "data:image/jpg" => "jpg",
        "data:image/png" => "png",
        "data:image/webp" => "webp",
        "data:image/gif" => "gif",
        _ => "bin",
    };
    let bytes = STANDARD
        .decode(encoded.as_bytes())
        .context("failed to decode inline image")?;
    if bytes.is_empty() {
        return Err(anyhow!("inline image is empty"));
    }
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return Err(anyhow!("inline image size exceeds 10MB"));
    }

    let stored = storage::save_bytes(
        state,
        sub_dir,
        &format!("inline-image.{ext}"),
        Some(mime_type.trim_start_matches("data:")),
        &bytes,
    )
    .await?;
    Ok(storage::public_url(&stored.relative_path))
}

fn find_next_inline_image_src(input: &str, from: usize) -> Option<(usize, char, usize)> {
    let haystack = input.get(from..)?;
    let src_pos = haystack.find("src=")?;
    let attr_start = from + src_pos;
    let quote_pos = attr_start + "src=".len();
    let quote = input[quote_pos..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return find_next_inline_image_src(input, quote_pos);
    }
    let data_start = quote_pos + quote.len_utf8();
    if !input[data_start..].starts_with(DATA_IMAGE_PREFIX) {
        return find_next_inline_image_src(input, data_start);
    }
    Some((attr_start, quote, data_start))
}
