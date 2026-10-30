use std::io::{self, Read, Seek, SeekFrom};

/// imagesize 需要回看文件头；deflate 流只缓存按需解压的前缀，不预先解压整页。
pub(crate) struct HeaderReader<R> {
    reader: R,
    prefix: Vec<u8>,
    position: u64,
    size: u64,
}

impl<R> HeaderReader<R> {
    pub(crate) fn new(reader: R, size: u64) -> Self {
        Self {
            reader,
            prefix: Vec::new(),
            position: 0,
            size,
        }
    }
}

impl<R: Read> Read for HeaderReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let end = self
            .position
            .saturating_add(buffer.len() as u64)
            .min(self.size);
        let missing = end.saturating_sub(self.prefix.len() as u64);
        self.reader
            .by_ref()
            .take(missing)
            .read_to_end(&mut self.prefix)?;
        if self.position >= self.prefix.len() as u64 {
            return Ok(0);
        }
        let start = self.position as usize;
        let count = buffer.len().min(self.prefix.len() - start);
        buffer[..count].copy_from_slice(&self.prefix[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl<R> Seek for HeaderReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.size) + i128::from(offset),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "图片头部的定位超出范围"))?;
        Ok(self.position)
    }
}
