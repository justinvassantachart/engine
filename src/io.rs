use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use wasmer_wasix::virtual_fs::{AsyncRead, AsyncSeek, AsyncWrite, FsError, Result, VirtualFile};

use crate::types::{StdoutMode, WorkerOut};

// ============================================================================
// Stdin
// ============================================================================

// Must match TypeScript StdinStream constants
const BUFFER_SIZE: u32 = 8 * 1024;
const HEADER_SIZE: u32 = 8;
const DATA_SIZE: u32 = BUFFER_SIZE - HEADER_SIZE;
const READ_IDX: u32 = 0;
const WRITE_IDX: u32 = 1;

#[derive(Debug)]
pub struct Stdin {
    indices: js_sys::Int32Array,
    data: js_sys::Uint8Array,
}

impl Stdin {
    pub fn new(stdin_buffer: &js_sys::SharedArrayBuffer) -> Self {
        // Create typed array views over the SharedArrayBuffer
        let indices = js_sys::Int32Array::new_with_byte_offset_and_length(
            stdin_buffer,
            0,
            2, // 2 i32s: read_index and write_index
        );
        let data = js_sys::Uint8Array::new_with_byte_offset_and_length(
            stdin_buffer,
            HEADER_SIZE,
            DATA_SIZE,
        );
        Self { indices, data }
    }

    /// Returns the number of bytes available to read
    fn available(&self) -> u32 {
        let read_idx = js_sys::Atomics::load(&self.indices, READ_IDX).unwrap_or(0) as u32;
        let write_idx = js_sys::Atomics::load(&self.indices, WRITE_IDX).unwrap_or(0) as u32;
        (write_idx.wrapping_sub(read_idx)) % DATA_SIZE
    }
}

impl AsyncWrite for Stdin {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot write to stdin",
        )))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

impl AsyncRead for Stdin {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut wasmer_wasix::virtual_fs::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let read_idx = js_sys::Atomics::load(&self.indices, READ_IDX).unwrap_or(0) as u32;
        let write_idx = js_sys::Atomics::load(&self.indices, WRITE_IDX).unwrap_or(0) as u32;

        // Calculate contiguous available bytes (no wrap-around handling)
        let available = if read_idx <= write_idx {
            write_idx - read_idx
        } else {
            DATA_SIZE - read_idx // Only read to end of buffer; next call handles wrapped part
        };

        if available == 0 {
            // Wait on the write index
            let _ = js_sys::Atomics::wait(&self.indices, WRITE_IDX, write_idx as i32);
            return Poll::Pending;
        }

        // Read contiguous chunk only
        let to_read = std::cmp::min(available as usize, buf.remaining());
        let slice = self.data.slice(read_idx, read_idx + to_read as u32);
        let mut temp = vec![0u8; to_read];
        slice.copy_to(&mut temp);
        buf.put_slice(&temp);

        // Update read index atomically (wraps naturally via modulo)
        let new_read_idx = ((read_idx as usize + to_read) % DATA_SIZE as usize) as i32;
        let _ = js_sys::Atomics::store(&self.indices, READ_IDX, new_read_idx);

        // Notify TypeScript that we consumed data
        let _ = js_sys::Atomics::notify(&self.indices, READ_IDX);

        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for Stdin {
    fn start_seek(self: Pin<&mut Self>, _position: io::SeekFrom) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek stdin",
        ))
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek stdin",
        )))
    }
}

impl VirtualFile for Stdin {
    fn last_accessed(&self) -> u64 {
        0
    }
    fn last_modified(&self) -> u64 {
        0
    }
    fn created_time(&self) -> u64 {
        0
    }
    fn size(&self) -> u64 {
        0
    }

    fn set_len(&mut self, _new_size: u64) -> Result<()> {
        Err(FsError::Unsupported)
    }

    fn unlink(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_special_fd(&self) -> Option<u32> {
        Some(0)
    }

    fn poll_read_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }

    fn poll_write_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot write to stdin",
        )))
    }
}

// ============================================================================
// Stdout
// ============================================================================

#[derive(Debug)]
pub struct Stdout {
    mode: StdoutMode,
}

impl Stdout {
    pub fn new(mode: StdoutMode) -> Self {
        Self { mode }
    }
}

impl AsyncWrite for Stdout {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        WorkerOut::Stdout {
            data: buf,
            mode: self.mode,
        }
        .send();
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

impl AsyncRead for Stdout {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut wasmer_wasix::virtual_fs::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read from stdout",
        )))
    }
}

impl AsyncSeek for Stdout {
    fn start_seek(self: Pin<&mut Self>, _position: io::SeekFrom) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek from stdout",
        ))
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek from stdout",
        )))
    }
}

impl VirtualFile for Stdout {
    fn last_accessed(&self) -> u64 {
        0
    }
    fn last_modified(&self) -> u64 {
        0
    }
    fn created_time(&self) -> u64 {
        0
    }
    fn size(&self) -> u64 {
        0
    }

    fn set_len(&mut self, _new_size: u64) -> Result<()> {
        Err(FsError::Unsupported)
    }

    fn unlink(&mut self) -> Result<()> {
        Err(FsError::Unsupported)
    }

    fn get_special_fd(&self) -> Option<u32> {
        Some(if let StdoutMode::Err = self.mode {
            2
        } else {
            1
        })
    }

    fn poll_read_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read from stdout",
        )))
    }

    fn poll_write_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(usize::MAX))
    }
}
