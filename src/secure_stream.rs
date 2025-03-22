use std::{io::{self, BufWriter, Read, Write}, net::TcpStream, sync::{Arc, Mutex}};

/// Wrapper around TcpStream that automatically hashes data sent and received through the socket
pub struct SecureStream{
    pub stream: TcpStream,
    hash: u64,
    read_offset: Arc<Mutex<u32>>,
    write_offset: Arc<Mutex<u32>>
}
impl SecureStream{
    pub fn new(stream: TcpStream) -> Self{
        Self{stream, hash: 0, read_offset: Arc::new(0.into()), write_offset: Arc::new(0.into())}
    }

    /// Sets a hash value for this SecureStream, returning itself 
    pub fn set_hash(mut self, hash: u64) -> Self{
        self.hash=hash;
        self
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
        Ok(Self{stream: self.stream.try_clone()?, hash: self.hash, read_offset: self.read_offset.clone(), write_offset: self.write_offset.clone()})
    }
}

impl Read for SecureStream{
    /// Wrapper around the TcpStream's read() function which unshuffles bytes based on the hash before reading. 
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error>{
        match self.read_offset.lock(){
            Ok(mut offset) => {
                let read_bytes = self.stream.read(buf)?;
                let mut bytes = [0u8; 8];
                let hash = self.hash.rotate_left(*offset * 8);
                for chunk in buf[..read_bytes].chunks_mut(8){
                    bytes[..chunk.len()].copy_from_slice(chunk);
                    let unshuffled = u64::from_be_bytes(bytes) ^ hash;
                    chunk.copy_from_slice(&unshuffled.to_be_bytes()[..chunk.len()]);
                }
                if read_bytes%8!=0{
                    *offset = (read_bytes as u32 + *offset) % 8;
                }
                Ok(read_bytes)
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string()))
        }
    }

    /// Wrapper around TcpStream's read_exact() function which decrypts bytes based on given hash before reading. 
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), io::Error>{
        match self.read_offset.lock(){
            Ok(mut offset) => {
                self.stream.read_exact(buf)?;
                let mut bytes = [0u8; 8];
                let hash = self.hash.rotate_left(*offset * 8);
                for chunk in buf.chunks_mut(8){
                    bytes[..chunk.len()].copy_from_slice(chunk);
                    let unshuffled = u64::from_be_bytes(bytes) ^ hash;
                    chunk.copy_from_slice(&unshuffled.to_be_bytes()[..chunk.len()]);
                }
                if buf.len()%8!=0{
                    *offset = (buf.len() as u32 + *offset) % 8;
                }
                Ok(())
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string()))
        }
    }
}

impl Write for SecureStream{
    /// Wrapper around the TcpStream's write() function which encrypts bytes based on the hash before writing. 
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error>{
        match self.write_offset.lock(){
            Ok(mut offset) => {
                let mut num_bytes_written = 0;
                let mut writer = BufWriter::new(&self.stream);
                let hash = self.hash.rotate_left(*offset * 8);
                for chunk in buf.chunks(8){
                    let mut bytes = [0u8; 8];
                    bytes[..chunk.len()].copy_from_slice(chunk);
                    let shuffled = u64::from_be_bytes(bytes) ^ hash;
                    writer.write_all(&shuffled.to_be_bytes()[..chunk.len()])?;
                    num_bytes_written+=chunk.len();
                }
                writer.flush()?;
                *offset = (*offset + num_bytes_written as u32) % 8;
                Ok(num_bytes_written)
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string()))
        }
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}