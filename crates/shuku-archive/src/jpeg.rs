//! imagesize 读不出尺寸的 JPEG 的后备解析。

use std::io::BufRead;

/// 顺序扫描 JPEG 的标记段，找到 SOF 段里的宽和高。
///
/// 有些 JPEG（常见于 Photoshop 导出后又改过 ICC 配置的图）在两个标记段之间夹着几个
/// 多余的字节。libjpeg 会跳过它们，imagesize 0.15 则报“Hit end of file before finding
/// size”。这里和 libjpeg 一样跳到下一个 0xFF，读到扫描数据（SOS）或文件尾还没有 SOF 时
/// 返回 `None`。
pub(crate) fn size(reader: impl BufRead) -> Option<(u32, u32)> {
    let mut bytes = reader.bytes().map_while(Result::ok);
    let mut next = || bytes.next();
    if (next()?, next()?) != (0xFF, 0xD8) {
        return None;
    }
    loop {
        let mut byte = next()?;
        while byte != 0xFF {
            byte = next()?;
        }
        let mut marker = next()?;
        while marker == 0xFF {
            marker = next()?;
        }
        match marker {
            // 0x00 只会出现在熵编码数据里，这里按杂字节处理；RST 和 TEM 没有长度。
            0x00 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let length = u16::from_be_bytes([next()?, next()?]);
        if length < 2 {
            return None;
        }
        // SOF0 到 SOF15，排除 DHT（C4）、JPG（C8）和 DAC（CC）。
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let _precision = next()?;
            let height = u16::from_be_bytes([next()?, next()?]);
            let width = u16::from_be_bytes([next()?, next()?]);
            return (width > 0 && height > 0).then_some((u32::from(width), u32::from(height)));
        }
        for _ in 2..length {
            next()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::size;

    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xFF, marker];
        bytes.extend(((body.len() + 2) as u16).to_be_bytes());
        bytes.extend(body);
        bytes
    }

    fn sof(width: u16, height: u16) -> Vec<u8> {
        let mut body = vec![8];
        body.extend(height.to_be_bytes());
        body.extend(width.to_be_bytes());
        body.extend([3, 1, 0x22, 0]);
        segment(0xC0, &body)
    }

    #[test]
    fn skips_stray_bytes_between_segments() {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(segment(0xE0, b"JFIF\0"));
        jpeg.extend(segment(0xE2, &[7; 900]));
        // 和实际遇到的文件一样，两个段之间夹着两个多余的字节。
        jpeg.extend([0x0C, 0x37]);
        jpeg.extend(segment(0xE2, &[7; 900]));
        jpeg.extend(segment(0xDB, &[0; 65]));
        jpeg.extend(sof(3000, 2097));
        jpeg.extend(segment(0xDA, &[0; 10]));
        assert_eq!(size(jpeg.as_slice()), Some((3000, 2097)));
    }

    #[test]
    fn reads_progressive_frames_and_fill_bytes() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xFF];
        jpeg.extend(segment(0xC4, &[0; 20]));
        let mut frame = sof(640, 480);
        frame[1] = 0xC2;
        jpeg.extend(frame);
        assert_eq!(size(jpeg.as_slice()), Some((640, 480)));
    }

    #[test]
    fn gives_up_without_a_frame_header() {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(segment(0xE0, b"JFIF\0"));
        assert_eq!(size(jpeg.as_slice()), None);
        jpeg.extend(segment(0xDA, &[0; 4]));
        jpeg.extend(sof(1, 1));
        assert_eq!(size(jpeg.as_slice()), None);
        assert_eq!(size(&b"\x89PNG\r\n"[..]), None);
        assert_eq!(size(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00][..]), None);
    }
}
