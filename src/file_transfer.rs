use std::{fs::File, io::{self, BufReader, BufWriter, Read, Write}};

use super::secure_stream::SecureStream;

pub fn send(stream: &mut SecureStream, file: File) -> Result<(), io::Error>{
    let mut buf_reader = BufReader::new(file);
    let mut buf = [0u8; 1024];
    let mut read_bytes = buf_reader.read(&mut buf)?;
    while read_bytes!=0{
        stream.write(&(read_bytes as u64).to_le_bytes())?;
        stream.write(&buf[..read_bytes])?;
        read_bytes = buf_reader.read(&mut buf)?;
    }
    stream.write(&[0u8; 8])?; // signify that file has finished being sent
    Ok(())
}

pub fn recv(stream: &mut SecureStream, file: File) -> Result<(), io::Error>{
    let mut buf_writer = BufWriter::new(file);
    let mut buf = [0u8; 1024];
    let mut size_buf = [0u8; 8];

    // before every <=1024 bytes, we expect 8 bytes representing the number of bytes being sent
    let mut read_bytes;
    let _ = stream.read(&mut size_buf)?;
    let mut size = u64::from_le_bytes(size_buf) as usize; // (u64::from_le_bytes(size_buf) as usize + 7) / 8 * 8;

    while size!=0{
        read_bytes = stream.read(&mut buf[..size.min(1024)])?;
        buf_writer.write_all(&buf[..read_bytes])?;
        
        size-=read_bytes;
        if size==0{
            stream.read(&mut size_buf)?;
            size = u64::from_le_bytes(size_buf) as usize;
        }
    }
    Ok(())
}