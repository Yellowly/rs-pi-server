use std::{io::{self, BufWriter, Read, Write}, net::TcpStream};

/// Wrapper around TcpStream that automatically hashes data sent and received through the socket
pub struct SecureStream{
    pub stream: TcpStream,
    hash: u64,
}
impl SecureStream{
    pub fn new(stream: TcpStream) -> Self{
        Self{stream, hash: 0}
    }

    /// Sets a hash value for this SecureStream, returning itself 
    pub fn set_hash(mut self, hash: u64) -> Self{
        self.hash=hash;
        self
    }

    /// Wrapper around the TcpStream's read() function which unshuffles bytes based on the hash before reading. 
    /// 
    /// Note that to maintain the alignment of hashed data, if buf is not a multiple of 8, this will throw away the remainder bytes.
    /// 
    /// If retaining all data is ncessary, you should ensure the given buf is a multiple of 8
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error>{
        let read_bytes = self.stream.read(buf)?;
        let mut bytes = [0u8; 8];
        for chunk in buf[..read_bytes].chunks_mut(8){
            bytes[..chunk.len()].copy_from_slice(chunk);
            let unshuffled = u64::from_le_bytes(bytes) ^ self.hash;
            chunk.copy_from_slice(&unshuffled.to_le_bytes()[..chunk.len()]);
        }
        if buf.len()%8!=0{
            let _ = self.stream.read(&mut bytes[buf.len()%8..]);
        }
        Ok(read_bytes)
    }

    /// Wrapper around the TcpStream's write() function which shuffles bytes based on the hash before writing. 
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error>{
        let mut num_bytes_written = 0;
        let mut writer = BufWriter::new(&self.stream);
        for chunk in buf.chunks(8){
            let mut bytes = [0u8; 8];
            bytes[..chunk.len()].copy_from_slice(chunk);
            let shuffled = u64::from_le_bytes(bytes) ^ self.hash;
            num_bytes_written += writer.write(&shuffled.to_le_bytes())?;
        }
        writer.flush()?;
        Ok(num_bytes_written)
    }

    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, io::Error>{
        self.stream.peer_addr()
    }
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, io::Error>{
        self.stream.local_addr()
    }
    pub fn shutdown(&self, how: std::net::Shutdown) -> Result<(), io::Error>{
        self.stream.shutdown(how)
    }
    pub fn set_read_timeout(&self, dur: Option<std::time::Duration>) -> io::Result<()>{
        self.stream.set_read_timeout(dur)
    }
    pub fn try_clone(&self) -> Result<Self, io::Error>{
        Ok(Self{stream: self.stream.try_clone()?, hash: self.hash})
    }
}