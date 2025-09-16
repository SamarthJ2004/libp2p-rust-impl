use common::EncryptedStream;
use muxer::Muxer;
use negotiation::negotiate_protocol;
use security::negotiate_security_protocol;
use std::{collections::HashSet, env, net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
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

    println!("[client] Starting multiplexing protocol negotiation...");
    let mux_protocol = negotiate_protocol(
        &mut stream,
        true,
        &supported_protocols().get("multiplexing").unwrap(),
    )
    .await
    .unwrap();
    println!("[client] Protocol negotiation complete");

    println!("[client] Starting protocol negotiation with {addr}");
    match mux_protocol.as_str() {
        "/mplex" => {
            // match negotiate_protocol(
            //     &mut stream,
            //     true,
            //     &supported_protocols().get("protocol").unwrap(),
            // )
            // .await
            // {
            //     Ok(protocol) => {
            //         println!(
            //             "[client] Protocol negotiation complete with {addr} and protocol: {protocol}"
            //         );
            // assume `arc_enc` is Arc<EncryptedStream> you constructed earlier:
            let arc_enc = Arc::new(stream);
            let mux = Muxer::new(arc_enc.clone(), true); // initiator = true
            mux.start_reader();

            // call the interactive loop:
            interactive_client_loop(mux).await;
            //         }
            //         Err(_) => {
            //             std::process::exit(1);
            //         }
            //     }
        }
        "/yamux" => {
            println!("[client] Unimplemented protocl");
            std::process::exit(1);
        }
        _ => {
            eprintln!("[client] no matching protocol found");
            std::process::exit(1);
        }
    }
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

    println!("[server] Starting multiplexing protocol negotiation...");
    let mux_protocol = negotiate_protocol(
        &mut stream,
        false,
        &supported_protocols().get("multiplexing").unwrap(),
    )
    .await
    .unwrap();
    println!("[server] Protocol negotiation complete");

    println!("[server] Starting protocol negotiation with {addr}");
    match mux_protocol.as_str() {
        "/mplex" => {
            // match negotiate_protocol(
            //     &mut stream,
            //     false,
            //     &supported_protocols().get("protocol").unwrap(),
            // )
            // .await
            // {
            //     Ok(protocol) => {
            //         println!(
            //             "[server] Protocol negotiation complete with {addr} and protocol :{protocol}"
            //         );
            //         ping_server(&mut stream, addr).await;
            //     }
            //     Err(_) => {
            //         std::process::exit(1);
            //     }
            // }

            let arc_stream = Arc::new(stream);
            let mux = Muxer::new(arc_stream.clone(), false); // false = responder (even ids)
            mux.start_reader();

            // accept loop
            tokio::spawn({
                let mux = mux.clone();
                async move {
                    loop {
                        if let Some((stream_id, proto, mut rx)) = mux.accept_stream().await {
                            tokio::spawn({
                                let mux = mux.clone();
                                async move {
                                    println!("Incoming stream {} proto={}", stream_id, proto);
                                    while let Some(bytes) = rx.recv().await {
                                        let s = String::from_utf8_lossy(&bytes);
                                        if s.trim().starts_with("PING") {
                                            let reply = s.replace("PING", "PONG");
                                            mux.send_data(stream_id, reply.as_bytes())
                                                .await
                                                .unwrap();
                                        }
                                    }
                                }
                            });
                        } else {
                            break;
                        }
                    }
                }
            });
        }
        "/yamux" => {
            println!("[server] Unimplemented protocl");
            std::process::exit(1);
        }
        _ => {
            eprintln!("[server] no matching protocol found");
            std::process::exit(1);
        }
    }
}

fn supported_protocols() -> HashMap<&'static str, Vec<&'static str>> {
    HashMap::from([
        ("security", vec!["/noise/xx", "/tls{unimplemented}"]),
        ("protocol", vec!["/ping/1.0.0"]),
        ("multiplexing", vec!["/yamux{unimplemented}", "/mplex"]),
    ])
}

pub async fn interactive_client_loop(mux: Arc<Muxer>) -> tokio::io::Result<()> {
    println!("Interactive client ready. Commands:");
    println!("  /open <protocol>      e.g. /open /ping/1.0.0");
    println!("  /send <id> <message>");
    println!("  /close <id>");
    println!("  /list");
    println!("  /quit");

    // keep a local set of open stream ids so we can list and validate
    let mut open_streams: HashSet<u32> = HashSet::new();

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, ' ');
        let cmd = parts.next().unwrap_or("");

        match cmd {
            "/open" => {
                let proto = parts.next().unwrap_or("").trim();
                if proto.is_empty() {
                    println!("Usage: /open <protocol>   e.g. /open /ping/1.0.0");
                    continue;
                }

                // open the stream
                match mux.open_stream(proto).await {
                    Ok((stream_id, rx)) => {
                        println!("[client] Opened stream id={}", stream_id);
                        open_streams.insert(stream_id);

                        // spawn a printer task to show responses arriving on this stream
                        tokio::spawn(async move {
                            let mut rx = rx;
                            while let Some(bytes) = rx.recv().await {
                                let s = String::from_utf8_lossy(&bytes);
                                println!("[s{}] <- {}", stream_id, s.trim_end());
                            }
                            println!("[s{}] receiver closed", stream_id);
                        });

                        // If protocol is ping, prompt user for a single ping message to send
                        if proto == "/ping/1.0.0" {
                            print!("Enter ping message to send on stream {}: ", stream_id);
                            // flush stdout before waiting
                            use std::io::Write;
                            std::io::stdout().flush().ok();

                            // read one more line (blocking on stdin via the same `lines` iterator
                            // is tricky because we've already borrowed it — so we read directly from stdin)
                            // Simpler: read next line from `lines` iterator.
                            match lines.next_line().await? {
                                Some(mut ping_line) => {
                                    ping_line = ping_line.trim().to_string();
                                    if !ping_line.is_empty() {
                                        let msg = format!("PING {}\n", ping_line);
                                        if let Err(e) =
                                            mux.send_data(stream_id, msg.as_bytes()).await
                                        {
                                            eprintln!("[client] send_data error: {}", e);
                                        } else {
                                            println!(
                                                "[client] -> sent on s{}: {}",
                                                stream_id,
                                                msg.trim_end()
                                            );
                                        }
                                    } else {
                                        println!("[client] no message entered; skip sending");
                                    }
                                }
                                None => {
                                    println!("[client] stdin closed");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[client] open_stream error: {}", e);
                    }
                }
            }

            "/send" => {
                // format: /send <id> <message>
                let id_str = parts.next().unwrap_or("");
                let msg = parts.next().unwrap_or("").trim();
                if id_str.is_empty() || msg.is_empty() {
                    println!("Usage: /send <id> <message>");
                    continue;
                }
                let sid: u32 = match id_str.parse() {
                    Ok(s) => s,
                    Err(_) => {
                        println!("Invalid stream id: {}", id_str);
                        continue;
                    }
                };
                if !open_streams.contains(&sid) {
                    println!("Stream {} not known/open", sid);
                    continue;
                }
                let payload = format!("{}\n", msg);
                if let Err(e) = mux.send_data(sid, payload.as_bytes()).await {
                    eprintln!("[client] send_data error: {}", e);
                } else {
                    println!("[client] -> sent on s{}: {}", sid, msg);
                }
            }

            "/close" => {
                let id_str = parts.next().unwrap_or("");
                if id_str.is_empty() {
                    println!("Usage: /close <id>");
                    continue;
                }
                let sid: u32 = match id_str.parse() {
                    Ok(s) => s,
                    Err(_) => {
                        println!("Invalid stream id: {}", id_str);
                        continue;
                    }
                };
                if !open_streams.contains(&sid) {
                    println!("Stream {} not known/open", sid);
                    continue;
                }
                if let Err(e) = mux.close_stream(sid).await {
                    eprintln!("[client] close_stream error: {}", e);
                } else {
                    println!("[client] closed stream {}", sid);
                    open_streams.remove(&sid);
                }
            }

            "/list" => {
                if open_streams.is_empty() {
                    println!("No open streams");
                } else {
                    println!("Open streams: {:?}", open_streams);
                }
            }

            "/quit" => {
                println!("Quitting.");
                break;
            }

            _ => {
                println!("Unknown command: {}", cmd);
                println!(
                    "Commands: /open /ping/1.0.0 | /send <id> <msg> | /close <id> | /list | /quit"
                );
            }
        }
    }

    Ok(())
}

async fn mux_client(stream: EncryptedStream) {
    let reader = BufReader::new(tokio::io::stdin());
    let mut lines = reader.lines();
    let arc_enc = Arc::new(stream);

    while let Ok(Some(line)) = lines.next_line().await {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "/open" => {
                if parts.len() < 2 {
                    println!("[Mux_client] Usage: /open <protocol>");
                    continue;
                }
                println!("[Mux_client] entered open");
                let mux = Muxer::new(arc_enc.clone(), true); // true = initiator (odd ids)
                mux.start_reader();

                // open stream 1
                let (s1, mut r1) = mux.open_stream("/ping/1.0.0").await.unwrap();
                tokio::spawn({
                    let mux = mux.clone();
                    async move {
                        for i in 0..10 {
                            let msg = format!("PING A-{}\n", i);
                            mux.send_data(s1, msg.as_bytes()).await.unwrap();
                            if let Some(bytes) = r1.recv().await {
                                println!("[client][s1] <- {}", String::from_utf8_lossy(&bytes));
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        mux.close_stream(s1).await.unwrap();
                    }
                });

                // open stream 3
                let (s2, mut r2) = mux.open_stream("/ping/1.0.0").await.unwrap();
                tokio::spawn({
                    let mux = mux.clone();
                    async move {
                        for i in 0..10 {
                            let msg = format!("PING B-{}\n", i);
                            mux.send_data(s2, msg.as_bytes()).await.unwrap();
                            if let Some(bytes) = r2.recv().await {
                                println!("[client][s2] <- {}", String::from_utf8_lossy(&bytes));
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        mux.close_stream(s2).await.unwrap();
                    }
                });
            }
            "/close" => {
                if parts.len() < 2 {
                    println!("[Mux_client] Usage: /close <id>");
                    continue;
                }
            }
            "/send" => {
                if parts.len() < 3 {
                    println!("[Mux_client] Usage: /send <id> <msg>");
                    continue;
                }
            }
            "/quit" => {
                println!("[Mux_client]Exiting client .....");
                break;
            }
            _ => {
                println!("[Mux_client]Unknown command");
            }
        }
    }
}
