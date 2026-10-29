//! Local generated-image references for durable history. Only the persistence
//! copy is externalized; requests hydrate the exact already-prepared bytes.
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::io::{self};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use ts_rs::TS;

use crate::models::ContentItem;
use crate::models::FunctionCallOutputContentItem;
use crate::models::ImageReference;
use crate::models::ResponseItem;

const MAX_BYTES: usize = codex_utils_image::MAX_PROMPT_IMAGE_INPUT_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SavedImage {
    pub saved_path: String,
    pub mime_type: String,
    pub sha256: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES as u64
    {
        return Err(io::Error::other(
            "saved image is not a bounded regular file",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("saved image is a reparse point"));
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(io::Error::other("saved image exceeds size limit"));
    }
    Ok(bytes)
}

fn dimensions(bytes: &[u8]) -> io::Result<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()
        .map_err(io::Error::other)
}

/// Return a reference for a canonical generated image paired with a saved-path
/// hint. Missing/moved originals and resized images get one immutable content-addressed
/// PNG/JPEG/etc file, rather than depending on future resize/encoder behavior.
pub fn externalize(
    image_url: &str,
    original: &Path,
    codex_home: &Path,
) -> io::Result<Option<ImageReference>> {
    if !original.is_absolute() {
        return Ok(None);
    }
    let Some((header, payload)) = image_url.split_once(',') else {
        return Ok(None);
    };
    let Some(mime) = header
        .strip_prefix("data:")
        .and_then(|header| header.strip_suffix(";base64"))
    else {
        return Ok(None);
    };
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return Ok(None),
    };
    if payload.len() > MAX_BYTES.saturating_mul(4).div_ceil(3).saturating_add(4) {
        return Ok(None);
    }
    let bytes = STANDARD.decode(payload).map_err(io::Error::other)?;
    if bytes.len() > MAX_BYTES || STANDARD.encode(&bytes) != payload {
        return Ok(None);
    }
    let (width, height) = dimensions(&bytes)?;
    let sha256 = digest(&bytes);
    let original_matches = fs::metadata(original)
        .is_ok_and(|metadata| metadata.len() == bytes.len() as u64)
        && read_bytes(original).is_ok_and(|original| original == bytes);
    let saved_path = if original_matches {
        original.canonicalize()?
    } else {
        let directory = codex_home.join("generated-image-history");
        fs::create_dir_all(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("invalid saved image directory"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::other("linked saved image directory"));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let destination = directory.join(format!("{sha256}.{extension}"));
        match read_bytes(&destination) {
            Ok(existing) if existing == bytes => {}
            Ok(_) => return Err(io::Error::other("saved image content-address collision")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
                temporary.write_all(&bytes)?;
                temporary.as_file().sync_all()?;
                if let Err(error) = temporary.persist_noclobber(&destination) {
                    // Another thread/process may have won the same content hash.
                    if read_bytes(&destination)? != bytes {
                        return Err(error.error);
                    }
                }
                #[cfg(unix)]
                fs::File::open(&directory)?.sync_all()?;
            }
            Err(error) => return Err(error),
        }
        destination.canonicalize()?
    };
    Ok(Some(ImageReference::Saved {
        saved_image: SavedImage {
            saved_path: saved_path.to_string_lossy().into_owned(),
            mime_type: mime.into(),
            sha256,
            byte_size: bytes.len() as u64,
            width,
            height,
        },
    }))
}

/// Missing/changed files are explicit errors, never silently omitted images or
/// paths sent to a remote provider. The original data URL is rebuilt byte-for-byte.
pub fn hydrate(reference: &ImageReference) -> io::Result<ImageReference> {
    let ImageReference::Saved { saved_image } = reference else {
        return Ok(reference.clone());
    };
    let path = Path::new(&saved_image.saved_path);
    if !path.is_absolute()
        || saved_image.byte_size > MAX_BYTES as u64
        || !matches!(
            saved_image.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        )
    {
        return Err(io::Error::other("invalid saved image reference"));
    }
    if fs::symlink_metadata(path)?.len() != saved_image.byte_size {
        return Err(io::Error::other(format!(
            "saved image changed: {}",
            path.display()
        )));
    }
    let bytes = read_bytes(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("saved image unavailable: {}: {error}", path.display()),
        )
    })?;
    if bytes.len() as u64 != saved_image.byte_size
        || digest(&bytes) != saved_image.sha256
        || dimensions(&bytes)? != (saved_image.width, saved_image.height)
    {
        return Err(io::Error::other(format!(
            "saved image changed: {}",
            path.display()
        )));
    }
    Ok(ImageReference::Inline {
        image_url: codex_utils_image::data_url_from_bytes(&saved_image.mime_type, &bytes),
    })
}

/// Requests only: history reads, paging, search and projection stay compact.
pub fn hydrate_response_items(items: &mut [ResponseItem]) -> io::Result<()> {
    let mut loaded = HashMap::<SavedImage, ImageReference>::new();
    let mut hydrate_one = |image: &mut ImageReference| -> io::Result<()> {
        if let ImageReference::Saved { saved_image } = image {
            let key = saved_image.clone();
            let hydrated = match loaded.get(&key) {
                Some(image) => image.clone(),
                None => {
                    let value = hydrate(image)?;
                    loaded.insert(key, value.clone());
                    value
                }
            };
            *image = hydrated;
        }
        Ok(())
    };
    for item in items {
        match item {
            ResponseItem::Message { content, .. } => {
                for part in content {
                    if let ContentItem::InputImage { image, .. } = part {
                        hydrate_one(image)?;
                    }
                }
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                if let Some(parts) = output.content_items_mut() {
                    for part in parts {
                        if let FunctionCallOutputContentItem::InputImage { image, .. } = part {
                            hydrate_one(image)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The generated-image tool emits this fixed first-line hint immediately after
/// the corresponding image, including through code-mode generatedImage().
pub fn parse_saved_image_path(text: &str) -> Option<&str> {
    let rest = text
        .lines()
        .next()?
        .strip_prefix("Generated images are saved to ")?
        .strip_suffix(" by default.")?;
    let splits: Vec<_> = rest.match_indices(" as ").map(|(index, _)| index).collect();
    let split = splits
        .iter()
        .copied()
        .find(|&index| {
            let directory = &rest[..index];
            let path = &rest[index + 4..];
            !directory.is_empty() && path.starts_with(directory)
        })
        .or_else(|| splits.last().copied())?;
    let path = rest[split + 4..].trim();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    #[test]
    fn saved_original_round_trips_exact_url_and_legacy_inline_is_unchanged() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("image.png");
        fs::write(&path, png()).unwrap();
        let url = codex_utils_image::data_url_from_bytes("image/png", &png());
        let original = ImageReference::Inline {
            image_url: url.clone(),
        };
        let saved = externalize(&url, &path, home.path()).unwrap().unwrap();
        assert_eq!(hydrate(&saved).unwrap(), original);
        assert_eq!(hydrate(&original).unwrap(), original);
        let encoded = serde_json::to_string(&saved).unwrap();
        assert!(!encoded.contains("base64"));
        assert!(encoded.contains("savedPath"));
        assert_eq!(
            serde_json::from_str::<ImageReference>(&encoded).unwrap(),
            saved
        );
        fs::remove_file(path).unwrap();
        assert!(hydrate(&saved).is_err());
    }
    #[test]
    fn prepared_bytes_are_deduplicated_and_changed_files_fail_explicitly() {
        let home = tempfile::tempdir().unwrap();
        let original = home.path().join("original.png");
        fs::write(&original, b"synthetic original differs from prepared PNG").unwrap();
        let url = codex_utils_image::data_url_from_bytes("image/png", &png());
        let first = externalize(&url, &original, home.path()).unwrap().unwrap();
        assert_eq!(
            first,
            externalize(&url, &original, home.path()).unwrap().unwrap()
        );
        assert_eq!(
            fs::read_dir(home.path().join("generated-image-history"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            hydrate(&first).unwrap(),
            ImageReference::Inline { image_url: url }
        );
        let ImageReference::Saved { saved_image } = first else {
            panic!("saved")
        };
        fs::write(&saved_image.saved_path, b"changed").unwrap();
        assert!(hydrate(&ImageReference::Saved { saved_image }).is_err());
    }
}
