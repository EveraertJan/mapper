use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::image_data::{ImageData, MediaError, load_image};

/// Loads images on worker threads so the render loop never stalls on disk
/// or decoding. Results are collected with [`Loader::poll`] once per frame.
pub struct Loader<K> {
    tx: Sender<(K, Result<ImageData, MediaError>)>,
    rx: Receiver<(K, Result<ImageData, MediaError>)>,
}

impl<K: Send + 'static> Default for Loader<K> {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}

impl<K: Send + 'static> Loader<K> {
    /// Starts loading `path`; the result arrives tagged with `key`.
    pub fn load(&self, key: K, path: PathBuf, max_dim: u32) {
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("image loader".into())
            .spawn(move || {
                let result = load_image(&path, max_dim);
                // The app may have quit meanwhile; nothing to do then.
                let _ = tx.send((key, result));
            });
        if let Err(err) = spawned {
            tracing::error!("could not start image loader thread: {err}");
        }
    }

    /// Finished loads since the last call.
    pub fn poll(&self) -> Vec<(K, Result<ImageData, MediaError>)> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_results_with_keys() {
        let dir = std::env::temp_dir().join(format!("rustmap-loader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();

        let loader = Loader::default();
        loader.load(7_u32, path, 1024);
        loader.load(8_u32, dir.join("missing.png"), 1024);

        let mut results = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while results.len() < 2 && std::time::Instant::now() < deadline {
            results.extend(loader.poll());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        results.sort_by_key(|(k, _)| *k);
        assert_eq!(results.len(), 2);
        let ok = results[0].1.as_ref().unwrap();
        assert_eq!((results[0].0, ok.width, ok.height), (7, 4, 2));
        assert!(results[1].1.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
