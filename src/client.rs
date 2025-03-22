use std::{env, fs::File, io::{self, ErrorKind, Read, Write}, net::TcpStream, str, sync::{Arc, Mutex}, time::{self, Duration, UNIX_EPOCH}};

use super::command_runner::ClientSession;
use super::secure_stream::SecureStream;
use super::file_transfer;

// PCG for random number generation
fn rng_32(seed: &mut u64) -> u32{
    let old_seed = *seed;
    *seed = seed.overflowing_mul(6364136223846793005u64).0 + 3217;
    let shifted = (((old_seed >> 18) ^ old_seed) >> 27) as u32;
    let rot = (old_seed >> 59) as u32;
    (shifted >> rot) | shifted << (rot.overflowing_neg().0 & 31)
}

fn rng_64(seed: &mut u64) -> u64{
    let left = rng_32(seed) as u64;
    let right = rng_32(seed) as u64;
    (left << 32) | right
}

// Unused - If a more scalable system for 'rspi' commands is necessary then this enum should be used to accomplish this
enum RsPiCmd{
    Procs,
    Adopt(usize),
    Orphan,
    None
}
impl TryFrom<&str> for RsPiCmd{
    type Error = io::Error;
    
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut temp = value.split_whitespace();

        let instructions = "RS-PI process manager commands:\n
                        procs\tlists processes managed by this app\n
                        adopt [process id or name]\tmake this client session take control of a running proccess\n
                        orphan\tgive control of this client's running process back to the server process manager.";

        if temp.next().ok_or(io::Error::new(ErrorKind::Other, "ERROR: Command does not start with 'rspi'"))?!="rspi"{
            Err(io::Error::new(ErrorKind::Other, "ERROR: Command does not start with 'rspi'"))
        }else{
            if let Some(cmd) = temp.next(){
                match cmd{
                    "procs" => Ok(Self::Procs),
                    "adopt" => 
                        match temp.next(){
                            Some(arg) => Ok(Self::Adopt(match arg.parse(){Ok(v) => v, Err(e) => return Err(io::Error::new(ErrorKind::Other, e))})), 
                            None => Err(io::Error::new(ErrorKind::Other, "Adopt a child process (listed by running 'rspi procs') into this remote client session."))
                        }
                    "orphan" => Ok(Self::Orphan),
                    _ => Err(io::Error::new(ErrorKind::Other, instructions))
                }
            }else{
                Err(io::Error::new(ErrorKind::Other, instructions))
            }
        }
    }
}

/// After receiving a connection from a client, this struct is used to store all the necessary data for the server to receive messages,
/// run the proper commands, and send the responses back to the client
pub struct Client{
    stream: SecureStream,
    session: ClientSession,
    processes: Arc<Mutex<Vec<ClientSession>>>
}
impl Client{
    /// Attempts to create a new Client struct to manage a connection to a client
    pub fn new(stream: TcpStream, processes: Arc<Mutex<Vec<ClientSession>>>) -> Result<Self, io::Error>{
        let mut stream = SecureStream::new(stream).set_hash(Self::get_hash().unwrap());

        // ensure password is correct before creating this client
        Self::check_password(&mut stream)?;

        let cwd = env::current_dir().unwrap();

        Ok(Self{stream, session: ClientSession::new(cwd)?, processes})
    }

    /// Gets the hash used to encrypt messages by checking for the "RSPI_SERVER_HASHKEY" enviorment variable
    fn get_hash() -> Result<u64, String>{
        let hashkey: u64 = match env::var("RSPI_SERVER_HASHKEY").unwrap_or(String::from("0")).parse(){
            Ok(n) => n, 
            Err(_) => return Err(String::from("RSPI_SERVER_HASHKEY enviorment variable cannoted be parsed to a u64!")),
        };
        let mut seed = time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() / 5;
        Ok(hashkey ^ rng_64(&mut seed))
    }
    
    /// Ensure the first message the client sends to us is the correct password, defined by the "RSPI_SERVER_PASS" enviorment variable
    fn check_password(stream: &mut SecureStream) -> Result<(), io::Error>{
        let pass: String = env::var("RSPI_SERVER_PASS").unwrap_or(String::from("Password"));

        let mut read_buffer: [u8; 64] = [0; 64];
        match stream.read(&mut read_buffer){
            Ok(msg_len) => {
                let received_msg = str::from_utf8(&read_buffer[0..msg_len]).unwrap_or_default().trim_end_matches('\0');
                if pass!=received_msg{
                    println!("Client {} failed password:\n{}", stream.peer_addr().unwrap().ip(),received_msg);
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    Err(io::Error::new(ErrorKind::PermissionDenied, format!("Client {} inputted incorrect password {}",stream.peer_addr().unwrap().ip(),received_msg)))
                }else{Ok(())}
            },
            Err(e) => {
                Err(e)
            }
        }
    }

    /// Runs this client, constantly checking for messages until the client disconnects
    pub fn run(mut self){
        let _ = self.stream.set_read_timeout(Some(Duration::new(0, 1000000)));
        println!("Connection established with {}, {}",self.stream.local_addr().unwrap().ip(),self.stream.peer_addr().unwrap().ip());
    
        let mut read_buffer: [u8; 1024] = [0; 1024];
    
        let mut running_process = false;
    
        self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes()).unwrap();
    
        loop{
            // first, check for messages sent by client and run the sent command
            match self.stream.read(&mut read_buffer){
                Ok(msg_len) => {
                    if msg_len==0 {break;}
                    let received_msg = str::from_utf8(&read_buffer[0..msg_len]).unwrap_or_default().trim_end_matches('\0');
                    // println!("Recieved response length {}: \n{}", msg_len, received_msg);
                    if self.session.has_child(){
                        running_process=true;
                        if received_msg.starts_with("SIG"){
                            let _ = self.session.signal(received_msg);
                        }else if received_msg == "rspi orphan"{
                            self.do_rspi_process_cmds(received_msg);
                        }else{
                            // println!("attempting to write stdin {} to proc {}",received_msg,self.session.cmd_name);
                            let _ = self.session.write_stdin(received_msg);
                        }
                    }else if received_msg.starts_with("SIG"){
                        break;
                    }else if received_msg.starts_with("rspi") && received_msg != "rspi orphan"{
                        if self.do_rspi_process_cmds(received_msg){
                            running_process = true;
                        }
                    }else{
                        match self.session.run_command(received_msg){
                            Ok(_) => running_process=true,
                            Err(e) => {let _ = self.stream.write(format!("{}\n{}$ ", e, self.session.path.display()).as_bytes());},
                        }
                    }
                },
                Err(s) => {
                    match s.kind(){
                        ErrorKind::WouldBlock | ErrorKind::TimedOut => (),
                        _ => {
                            println!("Something went wrong:\n{}\nClosing connection...",s);
                            break;
                        }
                    }
                },
            }

            // constantly read the output of the session and send it to the client
            if let Ok(()) = self.session.read_output(&mut self.stream) {}

            // send exit status if it has finished.
            else if running_process{
                // if the process has just ended, print the CWD, and exit status if child process failed.

                // this is really scuffed and i should really create a 'on child end' callback, but that
                // would require sending a closure to another thread which is headache i dont want to deal with
                if let Some(status) = self.session.exit_status(){
                    running_process = false;
                    if !status.success(){let _ = self.stream.write(format!("Process exited with status {}\n",status).as_bytes());}
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                }else if !self.session.has_child() {
                    running_process = false;
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                }
            }
        }
        self.session.kill();
        if self.session.close().is_err() { println!("Error closing session"); }
        println!("Client {} closed connection",self.stream.peer_addr().unwrap().ip());
        if let Err(e) = self.stream.shutdown(std::net::Shutdown::Both) { println!("Failed to shutdown connection\n{}", e); }
    }

    /// In addition to running standard terminal commands as child processes, the client should be able to transfer "ownership" 
    /// of processes to and from itself and the main server thread.
    /// 
    /// After the 'rspi' keyword is inputted, this function will get called to run the given command
    fn do_rspi_process_cmds(&mut self, received_msg: &str) -> bool{
        let mut temp = received_msg.split_whitespace();
        temp.next(); // ignore the "rspi"
        if let Some(cmd) = temp.next(){
            match cmd{
                "procs" => { // lists processes
                    if let Ok(procs) = self.processes.lock(){
                        let _ = self.stream.write((procs.iter()
                                .enumerate()
                                .map(|(id, proc)| 
                                    format!("{}\t{}\t{}",id, proc.cmd_name, if proc.has_child(){"running"}else{"not running"})
                                )
                            .collect::<Vec<String>>()
                            .join("\n")
                            +"\n").as_bytes());
                    }else{
                        let _ = self.stream.write(b"Could not find processes\n");
                    }
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                    false
                },
                "adopt" => { // client takes ownership of proccess
                    if let Some(arg) = temp.next(){
                        if let Ok(mut procs) = self.processes.lock(){
                            if let Ok(id) = arg.parse::<usize>(){
                                self.session.set_is_outputting(false);
                                let old_session = std::mem::replace(&mut self.session, procs.remove(id));
                                let _ = self.stream.write(format!("Successfully took control of process {}: {}\n",id,self.session.cmd_name).as_bytes());
                                if old_session.close().is_err(){
                                    let _ = self.stream.write(format!("Error closing old process\n").as_bytes());
                                }
                                self.session.set_is_outputting(true);
                                true
                            }else if let Some(id) = procs.iter().position(|proc| proc.cmd_name.eq_ignore_ascii_case(arg)){
                                self.session.set_is_outputting(false);
                                let old_session = std::mem::replace(&mut self.session, procs.remove(id));
                                let _ = self.stream.write(format!("Successfully took control of process {}: {}\n",id,self.session.cmd_name).as_bytes());
                                if old_session.close().is_err(){
                                    let _ = self.stream.write(format!("Error closing old process\n").as_bytes());
                                }
                                self.session.set_is_outputting(true);
                                let _ = self.stream.write(format!("Successfully took control of process {}: {}\n",id,self.session.cmd_name).as_bytes());
                                true
                            }else{
                                let _ = self.stream.write(format!("ERROR: Could not find process with id or name {}\n",arg).as_bytes());
                                let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                                false
                            }
                        }else{
                            false
                        }
                    }else{
                        let _ = self.stream.write(b"Adopt a child process (listed by running 'rspi procs') into this remote client session.\n");
                        let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                        false
                    }
                },
                "orphan" => { // client gives up ownership of proccess to the server
                    let path = self.session.path.clone();
                    let name = self.session.cmd_name.clone();
                    if let Ok(mut procs) = self.processes.lock(){
                        match ClientSession::new(path){
                            Ok(new_session) => {
                                self.session.set_is_outputting(false);
                                procs.push(std::mem::replace(&mut self.session, new_session));
                                let _ = self.stream.write(format!("Sucessfully gave control of {} to proccess manager with id {}\n",name,procs.len()-1).as_bytes());        
                            },
                            Err(e) => {
                                let _ = self.stream.write(format!("Unable to create new session:\n{}",e).as_bytes());        
                            }
                        }
                    }
                    false
                },
                "getfile" => {
                    if let Some(arg) = temp.next(){
                        let file_loc = self.session.path.join(arg);
                        let file = File::open(&file_loc);
                        match file{
                            Ok(f) => {
                                match file_transfer::send(&mut self.stream, f){
                                    Ok(_) => {let _ = self.stream.write(b"Successfully sent file to client!\n");},
                                    Err(e) => {let _ = self.stream.write(format!("Could not send file {}\n",e).as_bytes());}
                                };
                            },
                            Err(e) => {let _ = self.stream.write(format!("Could not find file at {}\n{}\n",file_loc.display(),e).as_bytes());}
                        }
                    }
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                    false
                },
                "sendfile" => {
                    if let Some(arg) = temp.next(){
                        let client_file_loc = std::path::PathBuf::from(arg);
                        let file_name = client_file_loc.file_name().unwrap_or(std::ffi::OsStr::new("new_file"));
                        let file_loc = self.session.path.join(file_name);
                        let file = File::create(&file_loc);
                        println!("attempting to recieve {}",file_loc.display());
                        match file{
                            Ok(f) => {
                                let _ = self.stream.set_read_timeout(Some(Duration::new(2, 0)));

                                match file_transfer::recv(&mut self.stream, f){
                                    Ok(_) => {let _ = self.stream.write(b"Successfully sent file to server!\n");},
                                    Err(e) => {let _ = self.stream.write(format!("Could not send file\n{}\n",e).as_bytes());}
                                };

                                let _ = self.stream.set_read_timeout(Some(Duration::new(0, 1000000)));
                            },
                            Err(e) => {let _ = self.stream.write(format!("Could not create file at {}\n{}\n",file_loc.display(),e).as_bytes());}
                        }
                    }
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                    false
                },
                _ => { // help instructions
                    let _ = self.stream.write(b"RS-PI process manager commands:\n
                        procs\tlists processes managed by this app\n
                        adopt [process id or name]\tmake this client session take control of a running proccess\n
                        orphan\tgive control of this client's running process back to the server process manager.\n");
                    let _ = self.stream.write(format!("{}$ ",self.session.path.display()).as_bytes());
                    false
                }
            }
        }else{
            false
        }
    }
}