use crate::{Error, Result};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::Cursor;

pub(crate) fn is_jxl(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\xff\x0a") || bytes.starts_with(b"\x00\x00\x00\x0cJXL \x0d\x0a\x87\x0a")
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    if is_jxl(bytes) {
        let mut decoder = jxl_oxide::integration::JxlDecoder::new(Cursor::new(bytes))?;
        decoder.set_limits(Limits::default())?;
        return Ok(DynamicImage::from_decoder(decoder)?);
    }
    if image::guess_format(bytes)? != ImageFormat::Avif {
        return Ok(ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()?
            .decode()?);
    }
    // image 的内置 AVIF 解码器需要系统 dav1d；这里使用 Rust 解码器。
    macro_rules! convert {
        ($pixels:expr, $variant:ident) => {{
            let (pixels, width, height) = $pixels.into_contiguous_buf();
            let buffer = image::ImageBuffer::from_raw(
                width as u32,
                height as u32,
                bytemuck::cast_slice(&pixels).to_vec(),
            )
            .ok_or_else(|| Error::File("AVIF 解码结果与图片尺寸不符".into()))?;
            DynamicImage::$variant(buffer)
        }};
    }
    Ok(match avif_decode::Decoder::from_avif(bytes)?.to_image()? {
        avif_decode::Image::Rgb8(pixels) => convert!(pixels, ImageRgb8),
        avif_decode::Image::Rgb16(pixels) => convert!(pixels, ImageRgb16),
        avif_decode::Image::Rgba8(pixels) => convert!(pixels, ImageRgba8),
        avif_decode::Image::Rgba16(pixels) => convert!(pixels, ImageRgba16),
        avif_decode::Image::Gray8(pixels) => convert!(pixels, ImageLuma8),
        avif_decode::Image::Gray16(pixels) => convert!(pixels, ImageLuma16),
    })
}
