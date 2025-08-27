use common::EncryptedStream;
use negotiation::negotiate_protocol;
use security::negotiate_security_protocol;
use std::{env, net::SocketAddr, sync::Arc};
use tokio::{
    io::{self, AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

use std::collections::HashMap;

const SERVER_ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = env::args().collect();

    println!("{:?}", args);

    if args.len() < 3 {
        if args.len() == 2 {
            args.push(SERVER_ADDR.to_string());
        } else {
            eprintln!("Usage: {} [server|client] <addr>", args[0]);
            std::process::exit(1);
        }
    }

    match args[1].to_lowercase().as_str() {
        "server" => run_server(&args[2]).await,
        "client" => run_client(&args[2]).await,
        _ => eprintln!("Invalid argument, server|client"),
    }
}

async fn run_server(addr: &str) {
    let stream = TcpListener::bind(addr)
        .await
        .expect("Unable to bind to the address");

    println!("Server listening on {addr}");

    loop {
        let (socket, addr) = stream.accept().await.expect("accept failed");
        println!("Accepted connection from {addr}");
        tokio::spawn(async move { handle_connection(socket, addr).await });
    }
}

async fn run_client(addr: &str) {
    let stream = TcpStream::connect(addr)
        .await
        .expect("Unable to connect to the address");

    println!("Connected to {addr}");

    let (reader, mut writer) = stream.into_split();
    let mut socket_reader = BufReader::new(reader);

    let transport = negotiate_security_protocol(
        &mut socket_reader,
        &mut writer,
        true,
        &supported_protocols(),
    )
    .await;

    let reader = socket_reader.into_inner();

    let mut stream = EncryptedStream {
        noise: transport,
        reader,
        writer,
    };

    negotiate_protocol(&mut stream, true, &supported_protocols()).await;

    let stream_arc = Arc::new(Mutex::new(stream));
    let stream_other = Arc::clone(&stream_arc);

    // read from the socket
    let socket_task = tokio::spawn(async move {
        loop {
            let mut lock = stream_arc.lock().await;
            let response = lock.recv().await.unwrap();
            match response.len() {
                0 => {
                    println!("Server closed connection");
                    break;
                }
                _ => {
                    println!("Server: {}", String::from_utf8_lossy(&response).trim());
                }
            }
        }
    });

    // read from the stdin and write to the socket
    let stdin_task = tokio::spawn(async move {
        let mut stdin_reader = BufReader::new(io::stdin());
        let mut input = String::new();

        loop {
            input.clear();
            let n = stdin_reader.read_line(&mut input).await.unwrap();
            if n == 0 {
                break;
            }

            input = input.trim().to_string();
            let msg = format!("PING {input}");
            let mut lock = stream_other.lock().await;
            if let Err(e) = lock.send(format!("{msg}\n").as_bytes()).await {
                eprintln!("Error writing to server: {e}");
                break;
            }
        }
    });

    let _ = tokio::join!(socket_task, stdin_task);
}

async fn handle_connection(socket: TcpStream, addr: SocketAddr) {
    let (reader, mut writer) = socket.into_split();
    let mut stream_reader = BufReader::new(reader);

    let transport = negotiate_security_protocol(
        &mut stream_reader,
        &mut writer,
        false,
        &supported_protocols(),
    )
    .await;

    let reader = stream_reader.into_inner();

    let mut stream = EncryptedStream {
        noise: transport,
        reader,
        writer,
    };

    negotiate_protocol(&mut stream, false, &supported_protocols()).await;

    loop {
        let response = stream.recv().await.unwrap();
        match response.len() {
            0 => {
                println!("Server closed connection");
                break;
            }
            _ => {
                println!("Server: {}", String::from_utf8_lossy(&response).trim());
            }
        }

        let line = String::from_utf8_lossy(&response);
        let msg = line.trim();
        println!("Received from {addr}: {}", msg);

        if msg.starts_with("PING") {
            let repsonse = msg.replace("PING", "PONG");
            if let Err(e) = stream.send(format!("{repsonse}\n").as_bytes()).await {
                eprintln!("Error writing to {addr}: {e}");
                break;
            }
        }
        if msg == "/multistream/1.0.0" {
            negotiate_protocol(&mut stream, true, &supported_protocols()).await;
        }
    }
}

fn supported_protocols() -> HashMap<&'static str, Vec<&'static str>> {
    HashMap::from([
        ("security", vec!["/noise/xx"]),
        ("protocol", vec!["/ping/1.0.0"]),
    ])
}
