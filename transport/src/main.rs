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
    println!("[main] Args: {:?}", args);

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
        _ => eprintln!("Invalid argument, expected: server|client"),
    }
}

async fn run_server(addr: &str) {
    let stream = TcpListener::bind(addr)
        .await
        .expect("Unable to bind to the address");

    println!("[server] Listening on {addr}");

    loop {
        let (socket, addr) = stream.accept().await.expect("accept failed");
        println!("[server] Accepted connection from {addr}");
        tokio::spawn(async move { handle_connection(socket, addr).await });
    }
}

async fn run_client(addr: &str) {
    let stream = TcpStream::connect(addr)
        .await
        .expect("Unable to connect to the address");

    println!("[client] Connected to {addr}");

    let (reader, mut writer) = stream.into_split();
    let mut socket_reader = BufReader::new(reader);

    println!("[client] Starting security negotiation...");
    let transport = negotiate_security_protocol(
        &mut socket_reader,
        &mut writer,
        true,
        &supported_protocols(),
    )
    .await;
    println!("[client] Security negotiation complete");

    let reader = socket_reader.into_inner();

    let mut stream = EncryptedStream {
        noise: Mutex::new(transport),
        reader: Mutex::new(reader),
        writer: Mutex::new(writer),
    };

    println!("[client] Starting protocol negotiation...");
    negotiate_protocol(&mut stream, true, &supported_protocols()).await;
    println!("[client] Protocol negotiation complete");

    let arc_stream = Arc::new(stream);
    let stream_clone = Arc::clone(&arc_stream);

    // task: read from the socket
    let socket_task = tokio::spawn(async move {
        loop {
            let response = arc_stream.recv().await.unwrap();
            match response.len() {
                0 => {
                    println!("[client] Server closed connection");
                    break;
                }
                _ => {
                    println!("[client] <- {}", String::from_utf8_lossy(&response).trim());
                }
            }
        }
    });

    // task: read from stdin and send to socket
    let stdin_task = tokio::spawn(async move {
        let mut stdin_reader = BufReader::new(io::stdin());
        let mut input = String::new();

        loop {
            input.clear();
            let n = stdin_reader.read_line(&mut input).await.unwrap();
            if n == 0 {
                println!("[client] stdin closed");
                break;
            }

            input = input.trim().to_string();
            let msg = format!("PING {input}");
            println!("[client] -> Sending: {msg}");
            if let Err(e) = stream_clone.send(format!("{msg}\n").as_bytes()).await {
                eprintln!("[client] Error writing to server: {e}");
                break;
            }
        }
    });

    let _ = tokio::join!(socket_task, stdin_task);
}

async fn handle_connection(socket: TcpStream, addr: SocketAddr) {
    println!("[server] Handling connection from {addr}");
    let (reader, mut writer) = socket.into_split();
    let mut stream_reader = BufReader::new(reader);

    println!("[server] Starting security negotiation with {addr}");
    let transport = negotiate_security_protocol(
        &mut stream_reader,
        &mut writer,
        false,
        &supported_protocols(),
    )
    .await;
    println!("[server] Security negotiation complete with {addr}");

    let reader = stream_reader.into_inner();

    let mut stream = EncryptedStream {
        noise: Mutex::new(transport),
        reader: Mutex::new(reader),
        writer: Mutex::new(writer),
    };

    println!("[server] Starting protocol negotiation with {addr}");
    negotiate_protocol(&mut stream, false, &supported_protocols()).await;
    println!("[server] Protocol negotiation complete with {addr}");

    loop {
        let response = stream.recv().await.unwrap();
        match response.len() {
            0 => {
                println!("[server] Client {addr} closed connection");
                break;
            }
            _ => {
                println!("[server] <- {}", String::from_utf8_lossy(&response).trim());
            }
        }

        let line = String::from_utf8_lossy(&response);
        let msg = line.trim();
        println!("[server] Received from {addr}: {msg}");

        if msg.starts_with("PING") {
            let reply = msg.replace("PING", "PONG");
            println!("[server] -> Sending: {reply}");
            if let Err(e) = stream.send(format!("{reply}\n").as_bytes()).await {
                eprintln!("[server] Error writing to {addr}: {e}");
                break;
            }
        }
    }
}

fn supported_protocols() -> HashMap<&'static str, Vec<&'static str>> {
    HashMap::from([
        ("security", vec!["/noise/xx"]),
        ("protocol", vec!["/ping/1.0.0"]),
    ])
}
