//! A write buffer for the serializers: small writes go into a `Vec` and reach the sink in large
//! chunks. A write into it never fails: the sink's first error is kept, nothing more is passed
//! on, and [`Chunks::finish`] returns that error. So the serializer's millions of small writes
//! cost what writes into a `Vec` cost (no error branch after each), which matters for the JSON
//! of a big table; after an error the rest of the document is still rendered, but not kept.

use std::io::{self, Write};

/// Size at which the gathered bytes are passed on.
const CHUNK: usize = 64 << 10;

pub(crate) struct Chunks<'a, W: Write + ?Sized> {
    buf: Vec<u8>,
    out: &'a mut W,
    error: Option<io::Error>,
}

impl<'a, W: Write + ?Sized> Chunks<'a, W> {
    pub(crate) fn new(out: &'a mut W) -> Self {
        Self {
            buf: Vec::with_capacity(CHUNK),
            out,
            error: None,
        }
    }

    /// Passes on the rest and returns the sink's first error; does not flush the sink.
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.spill();
        self.error.map_or(Ok(()), Err)
    }

    #[cold]
    fn spill(&mut self) {
        let buf = std::mem::take(&mut self.buf);
        self.pass(&buf);
        self.buf = buf;
        self.buf.clear();
    }

    /// Writes `b` to the sink unless it has already failed.
    fn pass(&mut self, b: &[u8]) {
        if self.error.is_none()
            && let Err(e) = self.out.write_all(b)
        {
            self.error = Some(e);
        }
    }

    /// A piece of a chunk or more goes to the sink directly, not through `buf`.
    #[cold]
    fn pass_large(&mut self, b: &[u8]) {
        self.spill();
        self.pass(b);
    }
}

impl<W: Write + ?Sized> Write for Chunks<'_, W> {
    #[inline]
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.write_all(b)?;
        Ok(b.len())
    }

    #[inline]
    fn write_all(&mut self, b: &[u8]) -> io::Result<()> {
        if b.len() >= CHUNK {
            self.pass_large(b);
            return Ok(());
        }
        self.buf.extend_from_slice(b);
        if self.buf.len() >= CHUNK {
            self.spill();
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
