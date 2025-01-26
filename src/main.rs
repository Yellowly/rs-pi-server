pub(self) mod secure_stream;
pub(self) mod command_runner;
pub(self) mod file_transfer;
mod client;

use std::{env, net::TcpListener, sync::{Arc, Mutex}, thread};
use command_runner::ClientSession;
use client::Client;

// Binds a listener to the address provided by either the "RSPI_SERVER_ADDR" enviorment variable or the first command line argument
fn main() {
    let args: Vec<String> = env::args().collect();
    let mut addr = env::var("RSPI_SERVER_ADDR").unwrap_or(String::from("127.0.0.1:8080"));
    if args.len()>1{
        addr = args[1].clone();
    }

    let listener = TcpListener::bind(&addr).unwrap();
    println!("Server started on {}",addr);

    let child_processes = Arc::new(Mutex::new(Vec::<ClientSession>::new()));

    for stream in listener.incoming() {
        match stream{
            Ok(stream) => {
                let child_processes_ref = child_processes.clone();
                thread::spawn(move || {if let Ok(mut client) = Client::new(stream, child_processes_ref){client.run()}});
            },
            Err(_) => {println!("Could not connect to client")},
        }
    }
}