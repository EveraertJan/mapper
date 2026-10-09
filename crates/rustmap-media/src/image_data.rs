use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("could not read {path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("could not play {path}: {reason}")]
    Video { path: PathBuf, reason: String },
}

/// A decoded image ready for upload: premultiplied RGBA8 with a full mip chain.
///
/// - **Premultiplied** so filtering never bleeds dark fringes from fully
///   transparent pixels into the edges of a PNG cut-out.
/// - **Mipmapped** because projection mapping often shrinks large images a lot,
///   and steep keystone angles shrink them unevenly; without mips that shimmers.
#[derive(Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    /// Level 0 first. Level `i` is `max(1, w >> i) x max(1, h >> i)`.
    pub mips: Vec<Vec<u8>>,
    /// Size before any downscale to fit the GPU.
    pub original_size: [u32; 2],
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageData")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("mips", &self.mips.len())
            .finish()
    }
}

impl ImageData {
    /// Builds from straight-alpha RGBA, downscaling so neither side exceeds `max_dim`.
    pub fn from_rgba(image: RgbaImage, max_dim: u32) -> Self {
        puffin::profile_function!();
        let original_size = [image.width(), image.height()];
        let mut base = image;
        let max_dim = max_dim.max(1);
        if base.width() > max_dim || base.height() > max_dim {
            let scale = max_dim as f32 / base.width().max(base.height()) as f32;
            let w = ((base.width() as f32 * scale).round() as u32).clamp(1, max_dim);
            let h = ((base.height() as f32 * scale).round() as u32).clamp(1, max_dim);
            tracing::info!(
                "image is {}x{}, larger than the GPU allows; scaling to {w}x{h}",
                base.width(),
                base.height()
            );
            base = image::imageops::resize(&base, w, h, FilterType::Lanczos3);
        }
        premultiply(&mut base);

        let (width, height) = base.dimensions();
        let levels = 32 - width.max(height).leading_zeros();
        let mut mips = Vec::with_capacity(levels as usize);
        let mut level = base;
        loop {
            let (w, h) = level.dimensions();
            let next = (w > 1 || h > 1).then(|| {
                image::imageops::resize(
                    &level,
                    (w / 2).max(1),
                    (h / 2).max(1),
                    FilterType::Triangle,
                )
            });
            mips.push(level.into_raw());
            match next {
                Some(n) => level = n,
                None => break,
            }
        }
        debug_assert_eq!(mips.len(), levels as usize);

        Self {
            width,
            height,
            mips,
            original_size,
        }
    }

    /// Width / height in pixels.
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}

fn premultiply(image: &mut RgbaImage) {
    for p in image.pixels_mut() {
        let a = u16::from(p[3]);
        if a != 255 {
            for c in &mut p.0[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
    }
}

/// Decodes an image file (blocking).
pub fn load_image(path: &Path, max_dim: u32) -> Result<ImageData, MediaError> {
    puffin::profile_function!();
    let image = image::open(path).map_err(|source| MediaError::Image {
        path: path.to_owned(),
        source,
    })?;
    let rgba = match image {
        DynamicImage::ImageRgba8(rgba) => rgba,
        other => other.to_rgba8(),
    };
    Ok(ImageData::from_rgba(rgba, max_dim))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_sizes() {
        let img = ImageData::from_rgba(RgbaImage::new(300, 100), 4096);
        let sizes: Vec<usize> = img.mips.iter().map(Vec::len).collect();
        // 300x100, 150x50, 75x25, 37x12, 18x6, 9x3, 4x1, 2x1, 1x1
        let expected: Vec<usize> = [
            (300, 100),
            (150, 50),
            (75, 25),
            (37, 12),
            (18, 6),
            (9, 3),
            (4, 1),
            (2, 1),
            (1, 1),
        ]
        .iter()
        .map(|(w, h)| w * h * 4)
        .collect();
        assert_eq!(sizes, expected);
    }

    #[test]
    fn premultiplies_alpha() {
        let mut rgba = RgbaImage::new(1, 1);
        rgba.put_pixel(0, 0, image::Rgba([200, 100, 50, 128]));
        let img = ImageData::from_rgba(rgba, 16);
        assert_eq!(&img.mips[0], &[100, 50, 25, 128]);
    }

    #[test]
    fn downscales_to_gpu_limit_keeping_aspect() {
        let img = ImageData::from_rgba(RgbaImage::new(1000, 500), 256);
        assert_eq!((img.width, img.height), (256, 128));
        assert_eq!(img.original_size, [1000, 500]);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let err = load_image(Path::new("/definitely/not/here.png"), 1024).unwrap_err();
        assert!(err.to_string().contains("not/here.png"));
    }
}
