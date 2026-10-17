#![allow(dead_code)]

use std::io::{BufRead, Read};

/// A `BufRead` that hands out at most `chunk` bytes per `fill_buf`, and
/// optionally stops the first chunk at `split` so that a read boundary
/// falls exactly there.
pub struct Chunked<'a> {
    data: &'a [u8],
    chunk: usize,
    split: Option<usize>,
}

impl<'a> Chunked<'a> {
    pub fn new(data: &'a [u8], chunk: usize) -> Self {
        Chunked {
            data,
            chunk,
            split: None,
        }
    }

    pub fn split_at(data: &'a [u8], split: usize) -> Self {
        Chunked {
            data,
            chunk: usize::MAX,
            split: Some(split),
        }
    }
}

impl Read for Chunked<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Chunked<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let mut n = self.chunk.min(self.data.len());
        if let Some(split) = self.split {
            n = n.min(split);
        }
        Ok(&self.data[..n])
    }

    fn consume(&mut self, n: usize) {
        self.data = &self.data[n..];
        if let Some(split) = self.split.as_mut() {
            *split -= n.min(*split);
            if *split == 0 {
                self.split = None;
            }
        }
    }
}
