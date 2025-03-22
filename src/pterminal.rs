use std::{ffi, fs::File, io::{self, BufReader, ErrorKind, Read}, os::fd::FromRawFd, process::{Child, Command}, sync::{Arc, Weak}};

unsafe extern "C"{
    fn close(fd: i32) -> i32;
    fn posix_openpt(oflag: i32) -> i32;
    fn grantpt(fd: i32) -> i32;
    fn unlockpt(fd: i32) -> i32;
    fn ptsname(fd: i32) -> *mut i8;
}

pub struct PseudoTerminal{
    master: Arc<File>,
    slave: Option<File>
}
impl PseudoTerminal{
    /// Creates a new pseudo-terminal which can be used to run processes
    pub fn new() -> Result<PseudoTerminal, io::Error>{
        let slavename: &str;
        let master = unsafe {
            let master_fd = posix_openpt(2);
            if master_fd==-1 { return Err(io::Error::last_os_error()) }

            // unlock pty while checking for errors
            if grantpt(master_fd) == -1 { close(master_fd); return Err(io::Error::last_os_error()) }
            if unlockpt(master_fd) == -1 { close(master_fd); return Err(io::Error::last_os_error()) }

            // get the name of the slave end of the pty
            slavename = ffi::CStr::from_ptr(ptsname(master_fd))
                            .to_str()
                            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            // return master
            File::from_raw_fd(master_fd)
        };
        let slave = File::create(slavename)?;
        Ok(Self{master: Arc::new(master), slave: Some(slave)})
    }

    /// Runs the command in this pseudo-terminal by redirecting its output
    /// 
    /// This will only redirect `stdout` and `stderr` to this pseudo-terminal.\
    /// It's recommended to write to `stdin` of the returned child directly if
    /// necessary
    pub fn run_cmd(&self, mut cmd: Command) -> io::Result<Child>{
        match &self.slave{
            Some(slave) => cmd.stdout(slave.try_clone()?).stderr(slave.try_clone()?).spawn(),
            None => Err(io::Error::from(ErrorKind::BrokenPipe))
        }
    }

    /// Create a buffer reader that will read a weak reference to this pseudo-terminal
    /// 
    /// Note that data may be lost if multiple readers try reading at the same time
    /// 
    /// If the `PseudoTerminal` that is being read from is dropped, then .read() will
    /// return EOF
    pub fn make_reader(&self) -> BufReader<TermReader>{
        BufReader::new(TermReader{master: Arc::downgrade(&self.master)})
    }
}

impl Read for PseudoTerminal{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.read(buf){
            Ok(len) => Ok(len),
            Err(e) => if Some(5) == e.raw_os_error(){ Ok(0) } else { Err(e) }
        }
    }
}

impl Read for &PseudoTerminal{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.clone().read(buf){
            Ok(len) => Ok(len),
            Err(e) => if Some(5) == e.raw_os_error(){ Ok(0) } else { Err(e) }
        }
    }
}

pub struct TermReader{
    master: Weak<File>
}
impl Read for TermReader{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.upgrade(){
            Some(mut f) => match f.read(buf){
                Ok(len) => Ok(len),
                Err(e) => if Some(5) == e.raw_os_error(){ Ok(0) } else { Err(io::Error::from(e.kind())) }
            },
            None => Ok(0)
        }
    }
}