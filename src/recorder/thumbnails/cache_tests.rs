use std::path::PathBuf;

use gpui::RenderImage;
use image::{Frame, ImageBuffer, Rgba};

use super::{ThumbnailCache, ThumbnailKey, ThumbnailSize};

fn key(bucket: u64) -> ThumbnailKey {
    ThumbnailKey::new(
        PathBuf::from("recording.mp4"),
        bucket,
        1_000_000,
        ThumbnailSize {
            width: 2,
            height: 2,
        },
    )
}

fn image(value: u8) -> std::sync::Arc<RenderImage> {
    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(2, 2, Rgba([value; 4]));
    std::sync::Arc::new(RenderImage::new([Frame::new(buffer)]))
}

#[test]
fn evicts_oldest_entry_at_capacity() {
    let mut cache = ThumbnailCache::new(2, 64);
    cache.insert(
        key(0),
        image(0),
        ThumbnailSize {
            width: 2,
            height: 2,
        },
    );
    cache.insert(
        key(1),
        image(1),
        ThumbnailSize {
            width: 2,
            height: 2,
        },
    );
    let _ = cache.get(&key(0));
    cache.insert(
        key(2),
        image(2),
        ThumbnailSize {
            width: 2,
            height: 2,
        },
    );

    assert!(cache.get(&key(0)).is_some());
    assert!(cache.get(&key(1)).is_none());
    assert!(cache.get(&key(2)).is_some());
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.bytes(), 32);
}

#[test]
fn rejects_entries_larger_than_memory_bound() {
    let mut cache = ThumbnailCache::new(4, 8);
    let result = cache.insert(
        key(0),
        image(0),
        ThumbnailSize {
            width: 2,
            height: 2,
        },
    );

    assert!(!result.inserted);
    assert_eq!(cache.len(), 0);
    assert_eq!(cache.bytes(), 0);
}
