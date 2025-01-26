use std::{collections::VecDeque, io::{self, BufReader, ErrorKind, Read, Write}, process::{Child, Command, ExitStatus, Stdio}, sync::{Arc, Mutex}, thread};

/// Represents a child process initiated by a client.
/// 
/// The client has the option to rescind control of the session back to the server, 
/// which causes the server to take ownership of the proccess and maintain it even after
/// the client disconnects. 
pub struct ClientSession{
    cmd: Command,
    pub cmd_name: String,
    process: Option<Child>,
    pub path: std::path::PathBuf,
    // stdin: Option<io::BufWriter<std::process::ChildStdin>>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Arc<Mutex<VecDeque<String>>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    is_running: Arc<Mutex<bool>>,
}
impl ClientSession{
    pub fn new(from_path: std::path::PathBuf) -> Self{
        ClientSession{cmd: Command::new(""), cmd_name: String::from("None"), process: None, path: from_path, stdin: None, stdout: Arc::default(), stderr: Arc::default(), is_running: Arc::default()}
    }
    /// Makes the client session run a command.
    /// 
    /// Returns Ok(None) if a new process was successfully started, and there was no prior process being run
    /// Returns Ok(Some(ExitStatus)) if a previous process exited successfully
    /// If the session is already running a process, then this will return Err(String) containing an error message
    pub fn run_command(&mut self, cmd: &str) -> Result<Option<ExitStatus>, std::io::Error>{
        let mut last_status = None;
        match self.process{
            Some(ref mut proc) => {
                match proc.try_wait(){
                    Ok(Some(status)) => last_status=Some(status),
                    Ok(None) => return Result::Err(std::io::Error::new(ErrorKind::Other,String::from("A process is already running and must end before a new one can be started."))),
                    Err(e) => return Result::Err(e)
                };
            },
            None => (),
        };
        let mut cmd_splitted = cmd.split_whitespace();
        let cmd_name = cmd_splitted.next().unwrap_or_default();
        if cmd_name=="cd"{
            self.change_dir(&cmd_splitted.collect::<Vec<&str>>().join(" "))?;
            return Result::Ok(last_status);
        }
        if cmd_name.as_bytes()[0]!=b'.'{
            // self.cmd = Command::new("sh");
            // self.cmd.current_dir(self.path.clone()).arg("-c").arg(format!("{} {}",cmd_name,&cmd_splitted.collect::<Vec<&str>>().join(" ")));
            self.cmd = Command::new(cmd_name);
            self.cmd.current_dir(self.path.clone()).args(cmd_splitted);
        }else{
            self.cmd = Command::new(cmd_name);
            self.cmd.current_dir(self.path.clone()).args(cmd_splitted);
            // println!("running exe {}",cmd_name);
        }
        self.cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // println!("curr dir: {}",match self.cmd.get_current_dir(){Some(p) => p.to_str().unwrap_or("Nonepath"), None => "Nonepath"});        
        self.process = match self.cmd.spawn(){
            Ok(mut proc) => {
                // self.stdin = Some(io::BufWriter::new(proc.stdin.take().expect("process has no stdin")));
                self.stdin = Some(proc.stdin.take().expect("process has no stdin"));
                self.stdout = self.spawn_buf_reader(Box::new(BufReader::new(proc.stdout.take().expect("process has no stdout"))), 64);
                self.stderr = self.spawn_buf_reader(Box::new(BufReader::new(proc.stderr.take().expect("process has no stderr"))), 16);
                Some(proc)
            },
            Err(e) => {
                return Result::Err(e);
            }
        };
        self.cmd_name = cmd_name.to_owned();
        return Result::Ok(last_status)
    }
    fn spawn_buf_reader(&mut self, mut out: Box<BufReader<dyn Read + std::marker::Send>>, max_len: usize) -> Arc<Mutex<VecDeque<String>>>{
        let res = Arc::new(Mutex::new(VecDeque::new()));
        let stdout = res.clone();
        let is_running = self.is_running.clone(); 
        thread::spawn(move || {let mut byte = [0u8]; let mut buf = Vec::new(); loop {
            match out.read(&mut byte){
                Ok(0) => { // EOF
                    match is_running.lock(){Ok(mut v) => {*v=false;}, Err(_) => ()};
                    break;
                },
                Ok(_) => {
                    if byte[0]==10{
                        // let mut stdout = me.stdout.lock().expect("Couldn't lock");
                        match stdout.lock(){
                            Ok(mut stdout) => {
                                if stdout.len() >= max_len{
                                    let _ = stdout.pop_front();
                                }
                                stdout.push_back(String::from_utf8(buf.clone()).unwrap_or(String::from("Error")));
                            },
                            Err(_) => ()
                        }
                        buf.clear();
                    }else{buf.push(byte[0])}
                },
                Err(e) => {
                    match stdout.lock(){
                        Ok(mut stdout) => {
                            if stdout.len() >= max_len{
                                let _ = stdout.pop_front();
                            }
                            stdout.push_back(e.to_string());
                        },
                        Err(_) => ()
                    }
                }
            }
        }});
        match self.is_running.lock(){Ok(mut v) => {*v=true;}, Err(_) => ()};
        res
    }
    pub fn kill(&mut self){
        match self.process{
            Some(ref mut proc) => {let _ = proc.kill();},
            None => (),
        }
    }
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
    // takes the exit status of the process
    pub fn exit_status(&mut self) -> Option<ExitStatus>{
        match self.process{
            Some(ref mut p) => match p.try_wait(){Ok(Some(e)) => Some(e), Ok(None) => None, Err(_) => None},
            None => None
        }
    }
    pub fn is_running(&self) -> bool{
        *self.is_running.lock().unwrap()
    }
    fn read_buf(&self, buf: &Arc<Mutex<VecDeque<String>>>) -> Option<String>{
        match buf.lock(){
            Ok(mut out) => {
                if out.is_empty(){None}
                else{Some(out.drain(..).collect::<Vec<String>>().join("\n"))}
            },
            Err(e) => {
                Some(e.to_string())
            }
        }
    }
    pub fn read_stdout(&self) -> Option<String>{
        self.read_buf(&self.stdout)
    }
    pub fn read_stderr(&self) -> Option<String>{
        self.read_buf(&self.stderr)
    }
    pub fn write_stdin(&mut self, buf: &str) -> Result<usize, io::Error>{
        match self.stdin.as_mut(){
            Some(p) => {
                p.write(format!("{}\n",buf).as_bytes())
            },
            None => Err(io::Error::new(ErrorKind::BrokenPipe, "Stdin does not exist")),
        }
    }
    pub fn change_dir(&mut self, loc: &str) -> Result<std::path::PathBuf, io::Error>{
        self.path = match self.path.join(loc).canonicalize(){Ok(p) => p, Err(e) => return Err(e)};
        return Ok(self.path.as_path().to_owned())
    }
    
}