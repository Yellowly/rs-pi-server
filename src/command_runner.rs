use std::{collections::VecDeque, io::{self, BufReader, ErrorKind, Read, Write}, process::{Child, Command, ExitStatus, Stdio}, sync::{atomic::{self, AtomicBool}, Arc, Mutex}, thread::{self, JoinHandle}};
use crate::circular_buffer::CircularBuffer;

use super::pterminal::PseudoTerminal;

/// Represents a child process initiated by a client.
/// 
/// The client has the option to rescind control of the session back to the server, 
/// which causes the server to take ownership of the proccess and maintain it even after
/// the client disconnects. 
pub struct ClientSession{
    term: PseudoTerminal,
    pub cmd_name: String,
    process: Option<Child>,
    pub path: std::path::PathBuf,
    // stdin: Option<io::BufWriter<std::process::ChildStdin>>,
    stdin: Option<std::process::ChildStdin>,
    output: Arc<Mutex<CircularBuffer<4096>>>,
    is_running: Arc<AtomicBool>,
    outputting: Arc<AtomicBool>,
    reader_handle: Option<JoinHandle<()>>
}
impl ClientSession{
    /// Create a new session for a client to run commands from
    pub fn new(from_path: std::path::PathBuf) -> io::Result<Self>{
        Ok({
            let mut res = ClientSession{
                term: PseudoTerminal::new()?, 
                cmd_name: String::from("None"), 
                process: None, 
                path: from_path, 
                stdin: None, 
                output: Arc::default(),
                is_running: Arc::new(AtomicBool::new(false)),
                outputting: Arc::new(AtomicBool::new(false)),
                reader_handle: None
            };
            res.reader_handle = Some(res.spawn_buf_reader(res.output.clone(), Box::new(res.term.make_reader()), 64));
            res
        })
    }

    /// Makes the client session run a command.
    /// 
    /// Returns Ok(None) if a new process was successfully started, and there was no prior process being run
    /// Returns Ok(Some(ExitStatus)) if a previous process exited successfully
    /// If the session is already running a process, then this will return Err(String) containing an error message
    pub fn run_command(&mut self, cmd: &str) -> Result<Option<ExitStatus>, std::io::Error>{
        // reap a previously-ran process if it has exited, raise an error if it is still running
        let mut last_status = None;
        match self.process{
            Some(ref mut proc) => {
                match proc.try_wait(){
                    Ok(Some(status)) => {
                        self.process = None;
                        last_status=Some(status)
                    },
                    Ok(None) => return Result::Err(std::io::Error::new(ErrorKind::Other,String::from("A process is already running and must end before a new one can be started."))),
                    Err(e) => return Result::Err(e)
                };
            },
            None => (),
        };
        
        // parse the current commnd
        let mut cmd_splitted = cmd.split_whitespace();
        let cmd_name = cmd_splitted.next().unwrap_or_default();

        // handle empty command and cd separately
        if cmd_name.is_empty(){
            return Err(io::Error::new(ErrorKind::Other, "Empty command"))
        }
        if cmd_name=="cd"{
            self.change_dir(&cmd_splitted.collect::<Vec<&str>>().join(" "))?;
            return Result::Ok(last_status);
        }

        let mut cmd;
        if cmd_name.as_bytes()[0]!=b'.'{
            cmd = Command::new(cmd_name);
            cmd.current_dir(self.path.clone()).args(cmd_splitted);
        }else{
            cmd = Command::new(cmd_name);
            cmd.current_dir(self.path.clone()).args(cmd_splitted);
        }
        cmd.stdin(Stdio::piped());
        
        self.process = match self.term.run_cmd(cmd){
            Ok(mut proc) => {                
                self.stdin = Some(proc.stdin.take().expect("process has no stdin"));
                Some(proc)
            },
            Err(e) => {
                return Result::Err(e);
            }
        };
        self.cmd_name = cmd_name.to_owned();
        return Result::Ok(last_status)
    }

    /// Separate thread used to read the internal pseudo-terminal running child processe
    fn spawn_buf_reader(&mut self, out: Arc<Mutex<CircularBuffer<4096>>>, mut src: Box<BufReader<dyn Read + std::marker::Send>>, max_len: usize) -> JoinHandle<()>{
        let is_running = self.is_running.clone(); 
        let is_outputting = self.outputting.clone();
        let handle = thread::spawn(move || {
            is_running.store(true, atomic::Ordering::Relaxed);
            let mut byte = [0u8]; let mut buf = Vec::new(); loop {
            match src.read(&mut byte){
                Ok(0) => { // EOF
                    is_running.store(false, atomic::Ordering::Relaxed);
                    break;
                },
                Ok(_) => {
                    buf.push(byte[0]);
                    // lock output so that the temporary 'buf' can write to it
                    if buf.len() > 4096{
                        match out.lock(){
                            Ok(mut output) => {
                                // if someone doesn't read from buffer often enough, data in the
                                // `output` buffer may get overwritten. this is fine if the client
                                // is not connected to a socket, because it allows us to clear out
                                // the piped output of the child process. but if this client session
                                // is outputting to a socket, then we don't want this, and would
                                // rather wait so that the client recieves all data. 

                                // this is a really stupid solution but its just for a silly raspberry pi
                                // home server so hopefully no one else is using it. 
                                if !is_outputting.load(atomic::Ordering::Relaxed) || 
                                        output.len() + buf.len() <= output.allocated_size(){
                                    let _ = output.write(&buf);
                                    buf.clear();
                                }
                            },
                            Err(e) => {
                                buf.push(10);
                                buf.extend_from_slice(e.to_string().as_bytes());
                                out.clear_poison();
                            },
                        }
                    // otherwise, just try to lock the output and write to it
                    }else{
                        match out.try_lock(){
                            Ok(mut output) => {
                                if !is_outputting.load(atomic::Ordering::Relaxed) ||
                                        output.len() + buf.len() <= output.allocated_size() {
                                    let _ = output.write(&buf);
                                    buf.clear();
                                }
                            },
                            Err(e) => {
                                match e{
                                    std::sync::TryLockError::Poisoned(poison_error) => {
                                        buf.extend_from_slice(poison_error.to_string().as_bytes());
                                        out.clear_poison();
                                    },
                                    std::sync::TryLockError::WouldBlock => (),
                                }
                            },
                        }
                    }
                },
                Err(e) => {
                    match out.lock(){
                        Ok(mut output) => {
                            let _ = output.write(e.to_string().as_bytes());
                        },
                        Err(_) => out.clear_poison(),
                    }
                }
            }
        }
        });
        handle
    }

    /// Sets whether the client session's internal terminal buffer should
    /// wait instead of overwritting existing data. 
    pub fn set_is_outputting(&self, val: bool){
        self.outputting.store(val, atomic::Ordering::Relaxed);
    }

    /// Aquire the lock for the 'is_running' bool and change its value\
    /// Returns whether the mutex was successfully aquired or not
    fn set_running_status(&self, val: bool) -> bool{
        self.is_running.store(val, atomic::Ordering::Relaxed);
        true
    }

    /// Kill the current running child process of the session
    pub fn kill(&mut self){
        match self.process{
            Some(ref mut proc) => {
                if proc.kill().is_ok() {
                    self.set_running_status(false);
                }
            },
            None => (),
        }
    }

    /// Signal to the current running child process
    pub fn signal(&self, sig: &str) -> Result<(), io::Error>{
        let a = match &self.process{
            Some(proc) => {
                let mut kill = Command::new("kill")
                    .args(["-s", sig, &proc.id().to_string()]).spawn()?;
                kill.wait()?;
            },
            None => return Err(io::Error::new(ErrorKind::Other,"No process to signal"))
        };
        Ok(a)
    }

    /// Consume the error status of the child process if it has ended, otherwise returns None
    pub fn exit_status(&mut self) -> Option<ExitStatus>{
        match self.process{
            Some(ref mut p) => match p.try_wait(){
                Ok(Some(e)) => {
                    self.process = None;
                    Some(e)
                }, 
                Ok(None) => None, 
                Err(_) => None
            },
            None => None
        }
    }

    /// Check if the session is running
    pub fn is_running(&self) -> bool{
        self.is_running.load(atomic::Ordering::Relaxed)
    }

    /// Check if there is a currently running child process being managed by this session
    pub fn has_child(&self) -> bool{
        self.process.is_some()
    }

    /// Used to read from stdout or stderr or child processes
    fn read_buf(&self, buf: &Arc<Mutex<VecDeque<String>>>) -> Option<String>{
        match buf.lock(){
            Ok(mut out) => {
                if out.is_empty(){None}
                else{Some(out.drain(..).collect::<String>())}
            },
            Err(e) => {
                Some(e.to_string())
            }
        }
    }

    /// Reads the output of the session to a buffer
    /// 
    /// If the output's mutex is poisoned, returns io::ErrorKind::Other\
    /// If the output is empty, returns io::ErrorKind::UnexpectedEof
    pub fn read_output<T: Write>(&self, to: &mut T) -> io::Result<()>{
        match self.output.lock(){
            Ok(mut out) => {
                if !out.is_empty(){ let _ = out.write_to(to); Ok(())}
                else { Err(io::Error::new(ErrorKind::UnexpectedEof, String::from("Output is empty"))) }
            },
            Err(e) => {
                self.output.clear_poison();
                Err(io::Error::new(ErrorKind::Other, e.to_string()))
            }
        }
    }

    /// Write to the stdin of the currently running child process
    pub fn write_stdin(&mut self, buf: &str) -> Result<usize, io::Error>{
        match self.stdin.as_mut(){
            Some(p) => {
                p.write(format!("{}\n",buf).as_bytes())
            },
            None => Err(io::Error::new(ErrorKind::BrokenPipe, "Stdin does not exist")),
        }
    }

    /// Change the directory this client session is running from
    pub fn change_dir(&mut self, loc: &str) -> Result<std::path::PathBuf, io::Error>{
        self.path = match self.path.join(loc).canonicalize(){Ok(p) => p, Err(e) => return Err(e)};
        return Ok(self.path.as_path().to_owned())
    }

    /// Closes the terminal associated with this client session and joins the thread reading the terminal
    /// 
    /// Important to do this before dropping to join the thread created by this session
    /// 
    /// This is a horrible solution but according to [stack overflow](https://stackoverflow.com/questions/41331577/joining-a-thread-in-a-method-that-takes-mut-self-like-drop-results-in-cann/42791007#42791007)
    /// joining threads in a destructor is bad
    pub fn close(self) -> std::thread::Result<()>{
        drop(self.term);

        match self.reader_handle{
            Some(handle) => handle.join(),
            None => Ok(())
        }
    }
}