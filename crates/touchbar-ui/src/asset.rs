use sha2::Digest;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context as _, Result, bail};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg,
};

use crate::Image;

const MAX_SVG_BYTES: usize = 256 * 1024;
const MAX_RASTER_EDGE: u32 = 2048;
const MAX_RASTER_PIXELS: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct SvgAsset {
    pub id: u64,
    pub revision: u64,
    source: Arc<str>,
}

impl SvgAsset {
    pub fn new(id: u64, revision: u64, source: impl Into<Arc<str>>) -> Result<Self> {
        let source = source.into();
        if source.is_empty() {
            bail!("SVG source cannot be empty");
        }
        if source.len() > MAX_SVG_BYTES {
            bail!("SVG source exceeds the {MAX_SVG_BYTES}-byte limit");
        }
        Ok(Self {
            id,
            revision,
            source,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialOrd, PartialEq)]
struct RasterKey {
    content: [u8; 32],
    width: u32,
    height: u32,
}

/// Bounded content-addressed SVG raster cache. Theme colors are deliberately not
/// part of the key: symbolic assets are tinted later by the GLES image shader.
pub struct SvgRasterizer {
    cache: BTreeMap<RasterKey, Image>,
    insertion_order: VecDeque<RasterKey>,
    capacity: usize,
}

impl Default for SvgRasterizer {
    fn default() -> Self {
        Self::new(64)
    }
}

impl SvgRasterizer {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: BTreeMap::new(),
            insertion_order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn rasterize(&mut self, asset: &SvgAsset, width: u32, height: u32) -> Result<Image> {
        validate_dimensions(width, height)?;
        let key = RasterKey {
            content: sha2::Sha256::digest(asset.source().as_bytes()).into(),
            width,
            height,
        };
        if let Some(image) = self.cache.get(&key) {
            return Ok(image.clone());
        }

        // Symbolic SVG is self-contained vector data. usvg's defaults read local
        // files and decode embedded images outside the loader's IO/size budget.
        let image_resource = AtomicBool::new(false);
        let options = usvg::Options {
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_string: Box::new(|_, _| {
                    image_resource.store(true, Ordering::Relaxed);
                    None
                }),
                resolve_data: Box::new(|_, _, _| {
                    image_resource.store(true, Ordering::Relaxed);
                    None
                }),
            },
            ..Default::default()
        };
        let tree =
            usvg::Tree::from_str(asset.source(), &options).context("parse static SVG asset")?;
        if image_resource.load(Ordering::Relaxed) {
            bail!("image resources are not supported in symbolic SVG; declare a PNG asset instead");
        }
        let source = tree.size();
        let scale_x = width as f32 / source.width();
        let scale_y = height as f32 / source.height();
        let scale = scale_x.min(scale_y);
        let offset_x = (width as f32 - source.width() * scale) * 0.5;
        let offset_y = (height as f32 - source.height() * scale) * 0.5;
        let transform = Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);
        let mut pixmap = Pixmap::new(width, height).context("allocate SVG raster target")?;
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let mut pixels = pixmap.take();
        unpremultiply_rgba(&mut pixels);
        let image = Image::rgba8(
            raster_image_id(asset.id, width, height),
            asset.revision,
            width,
            height,
            pixels,
        )?;
        self.insert(key, image.clone());
        Ok(image)
    }

    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.insertion_order.clear();
    }

    fn insert(&mut self, key: RasterKey, image: Image) {
        while self.cache.len() >= self.capacity {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            self.cache.remove(&oldest);
        }
        self.insertion_order.push_back(key);
        self.cache.insert(key, image);
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        bail!("SVG raster dimensions must be nonzero");
    }
    if width > MAX_RASTER_EDGE
        || height > MAX_RASTER_EDGE
        || u64::from(width) * u64::from(height) > MAX_RASTER_PIXELS
    {
        bail!("SVG raster dimensions exceed the toolkit resource limit");
    }
    Ok(())
}

fn raster_image_id(asset_id: u64, width: u32, height: u32) -> u64 {
    let dimensions = (u64::from(width) << 32) | u64::from(height);
    asset_id.rotate_left(17) ^ dimensions.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

fn unpremultiply_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
        <path fill="#ffffff" d="M1 1h8v8H1z"/>
    </svg>"##;

    #[test]
    fn rasterizes_static_svg_and_reuses_the_cached_image() {
        let asset = SvgAsset::new(7, 3, ICON).unwrap();
        let mut rasterizer = SvgRasterizer::default();
        let first = rasterizer.rasterize(&asset, 32, 32).unwrap();
        let second = rasterizer.rasterize(&asset, 32, 32).unwrap();
        assert_eq!(first, second);
        assert_eq!(rasterizer.cached_count(), 1);
        assert!(
            first
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
    }

    #[test]
    fn cache_distinguishes_size_and_remains_bounded() {
        let mut rasterizer = SvgRasterizer::new(2);
        let first = SvgAsset::new(8, 1, ICON).unwrap();
        let revised = SvgAsset::new(8, 2, ICON).unwrap();
        rasterizer.rasterize(&first, 16, 16).unwrap();
        rasterizer.rasterize(&first, 24, 24).unwrap();
        rasterizer.rasterize(&revised, 16, 16).unwrap();
        assert_eq!(rasterizer.cached_count(), 2);
    }

    #[test]
    fn rejects_empty_oversized_and_invalid_assets() {
        assert!(SvgAsset::new(1, 1, "").is_err());
        let asset = SvgAsset::new(1, 1, ICON).unwrap();
        assert!(SvgRasterizer::default().rasterize(&asset, 0, 10).is_err());
        assert!(
            SvgRasterizer::default()
                .rasterize(&asset, 4096, 10)
                .is_err()
        );
        let invalid = SvgAsset::new(2, 1, "<svg>").unwrap();
        assert!(
            SvgRasterizer::default()
                .rasterize(&invalid, 10, 10)
                .is_err()
        );
    }

    fn rejects_image_reference(href: &str, filter: bool) {
        let element = if filter {
            format!(
                r#"<defs><filter id="f"><feImage href="{href}"/></filter></defs><rect width="10" height="10" filter="url(#f)"/>"#
            )
        } else {
            format!(r#"<image href="{href}" width="10" height="10"/>"#)
        };
        let asset = SvgAsset::new(
            1,
            1,
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{element}</svg>"#
            ),
        )
        .unwrap();
        let error = SvgRasterizer::default()
            .rasterize(&asset, 10, 10)
            .expect_err("symbolic SVG must not resolve image resources");
        assert!(error.to_string().contains("image resources"), "{error}");
    }

    #[test]
    fn symbolic_svg_rejects_external_and_embedded_images() {
        let directory = tempfile::tempdir().unwrap();
        let external = directory.path().join("outside.svg");
        std::fs::write(&external, ICON).unwrap();
        for filter in [false, true] {
            rejects_image_reference(external.to_str().unwrap(), filter);
            rejects_image_reference(
                "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==",
                filter,
            );
        }
    }

    #[test]
    fn symbolic_svg_rejects_fifo_without_blocking() {
        const CHILD: &str = "TOUCHBAR_SVG_FIFO_TEST";
        if let Some(path) = std::env::var_os(CHILD) {
            rejects_image_reference(path.to_str().unwrap(), false);
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("image.svg");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "asset::tests::symbolic_svg_rejects_fifo_without_blocking",
            ])
            .env(CHILD, &fifo)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "SVG child failed: {status}");
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("SVG rasterizer blocked reading an external FIFO");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn symbolic_svg_keeps_local_vector_references() {
        let asset = SvgAsset::new(1, 1, r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><path id="shape" d="M1 1h8v8H1z"/></defs><use href="#shape"/></svg>"##).unwrap();
        let image = SvgRasterizer::default().rasterize(&asset, 10, 10).unwrap();
        assert!(image.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    }
}
