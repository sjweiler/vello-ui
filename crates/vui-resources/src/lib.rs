//! Background-loadable resources for VUI applications.

use std::{
    collections::HashMap,
    hash::Hash,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};
use vui_core::{RasterImage, TaskContext, TreeNode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    pub byte_len: u64,
}

/// Decodes PNG or JPEG bytes into renderer-neutral RGBA8 pixels.
///
/// # Errors
///
/// Returns an error when the encoded bytes are unsupported or malformed.
pub fn decode_image(bytes: &[u8]) -> Result<RasterImage, String> {
    let decoded = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let rgba = decoded.to_rgba8();
    RasterImage::new(rgba.width(), rgba.height(), rgba.into_raw())
}

/// Reads and decodes an image file cooperatively on a task worker.
///
/// # Errors
///
/// Returns an error for cancellation, file I/O, or decoding failures.
pub fn load_image_file(path: impl AsRef<Path>, task: &TaskContext) -> Result<RasterImage, String> {
    if task.is_cancelled() {
        return Err("cancelled".to_owned());
    }
    let bytes = std::fs::read(path.as_ref()).map_err(|error| error.to_string())?;
    task.report(Some(0.5), "decoding".to_owned());
    if task.is_cancelled() {
        return Err("cancelled".to_owned());
    }
    decode_image(&bytes)
}

/// Reads one directory level and reports progress as entries are inspected.
///
/// # Errors
///
/// Returns an error for cancellation, directory I/O, or metadata failures.
pub fn read_directory(
    path: impl AsRef<Path>,
    task: &TaskContext,
) -> Result<Vec<DirectoryEntry>, String> {
    let entries = std::fs::read_dir(path.as_ref())
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let total = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let mut output = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate() {
        if task.is_cancelled() {
            return Err("cancelled".to_owned());
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        output.push(DirectoryEntry {
            path: entry.path(),
            name: name.clone(),
            is_directory: metadata.is_dir(),
            byte_len: metadata.len(),
        });
        let completed = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        let fraction = (total > 0).then(|| f64::from(completed) / f64::from(total));
        task.report(fraction, name);
    }
    output.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(output)
}

/// Converts a directory load into nodes suitable for `TreeView` lazy children.
///
/// # Errors
///
/// Returns the same errors as [`read_directory`].
pub fn load_tree_children(
    path: impl AsRef<Path>,
    task: &TaskContext,
) -> Result<Vec<TreeNode>, String> {
    read_directory(path, task).map(|entries| {
        entries
            .into_iter()
            .map(|entry| {
                let key = entry.path.to_string_lossy().into_owned();
                if entry.is_directory {
                    TreeNode::unloaded(key, entry.name)
                } else {
                    TreeNode::leaf(key, entry.name)
                }
            })
            .collect()
    })
}

#[derive(Debug)]
pub struct ResourceCache<Key, Value> {
    values: HashMap<Key, Weak<Value>>,
}

impl<Key, Value> Default for ResourceCache<Key, Value> {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
        }
    }
}

impl<Key: Eq + Hash, Value> ResourceCache<Key, Value> {
    pub fn insert(&mut self, key: Key, value: &Arc<Value>) {
        self.values.insert(key, Arc::downgrade(value));
    }

    #[must_use]
    pub fn get(&self, key: &Key) -> Option<Arc<Value>> {
        self.values.get(key).and_then(Weak::upgrade)
    }

    pub fn get_or_insert_with(&mut self, key: Key, create: impl FnOnce() -> Value) -> Arc<Value> {
        if let Some(value) = self.get(&key) {
            return value;
        }
        let value = Arc::new(create());
        self.insert(key, &value);
        value
    }

    pub fn prune(&mut self) {
        self.values.retain(|_, value| value.strong_count() > 0);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values
            .values()
            .filter(|value| value.strong_count() > 0)
            .count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

    #[test]
    fn decodes_png_into_rgba_resource() {
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded)
            .write_image(&[255, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .expect("encode test PNG");
        let image = decode_image(&encoded).expect("decode test PNG");
        assert_eq!(image.width(), 1);
        assert_eq!(image.height(), 1);
        assert_eq!(image.pixels().as_ref(), &[255, 0, 0, 255]);
    }

    #[test]
    fn weak_cache_reuses_live_values_and_prunes_released_values() {
        let mut cache = ResourceCache::<String, String>::default();
        let first = cache.get_or_insert_with("icon".to_owned(), || "decoded".to_owned());
        let second = cache.get_or_insert_with("icon".to_owned(), || "duplicate".to_owned());
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.len(), 1);
        drop(first);
        drop(second);
        cache.prune();
        assert!(cache.is_empty());
    }

    #[test]
    fn invalid_image_bytes_return_an_error() {
        assert!(decode_image(b"not an image").is_err());
    }
}
