use std::io::{self, Read, Write};

/// Circular buffer with fixed size
pub struct CircularBuffer<const N: usize>{
    data: [u8; N],
    head: usize,
    len: usize
}

impl<const N: usize> CircularBuffer<N>{
    pub const fn new() -> Self{
        Self{data: [0; N], head: 0, len: 0}
    }

    /// Returns the current number of bytes that have been written to this buffer
    pub fn len(&self) -> usize{
        self.len
    }

    /// Writes the entire contents of this circular buffer to a writer
    pub fn write_to<T: Write>(&mut self, to: &mut T) -> io::Result<()>{
        to.write_all(&self.data[self.head..N.min(self.head + self.len)])?;
        if self.head + self.len > N {
            to.write_all(&self.data[..self.len])?;
        }
        self.head = (self.head + self.len) % N;
        self.len = 0;
        Ok(())
    }

    pub fn is_empty(&self) -> bool{
        self.len == 0
    }

    pub const fn allocated_size(&self) -> usize{
        N
    }
}

impl <const N: usize> Read for CircularBuffer<N>{
    /// Reads bytes from this buffer into buf
    /// 
    /// If the buffer is empty, returns a WouldBlock error
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.is_empty() { return Err(io::Error::new(io::ErrorKind::WouldBlock, String::from("Buffer is empty"))) }
        let size = self.len.min(buf.len());
        let first_half = size.min(N-self.head);
        buf[..first_half].copy_from_slice(&self.data[self.head..self.head+first_half]);
        if first_half < size{
            buf[first_half..size].copy_from_slice(&self.data[..size-first_half]);
        }
        self.len -= size;
        self.head = (self.head + size) % N;
        Ok(size)
    }
}

impl <const N: usize> Write for CircularBuffer<N>{
    /// Writes bytes from `buf` into this buffer
    /// 
    /// Write will always write up to `N` bytes, where `N` is the initial allocated
    /// size of this buffer. 
    /// 
    /// Writes after the buffer reaches length `N` will cause previously written data
    /// to get overwritten. 
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let tail = (self.head + self.len) % N;
        let size = N.min(buf.len());
        let first_half = size.min(N-tail);
        self.data[tail..(tail+first_half)].copy_from_slice(&buf[..first_half]);
        if first_half < size{
            self.data[..size-first_half].copy_from_slice(&buf[first_half..size]);
        }
        self.len = N.min(self.len + size);
        Ok(size)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl <const N: usize> Default for CircularBuffer<N>{
    fn default() -> Self {
        Self::new()
    }
}