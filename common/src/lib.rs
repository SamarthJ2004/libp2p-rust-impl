use std::{net::SocketAddr, sync::Arc};

use snow::TransportState;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::Mutex,
};

#[derive(Debug)]
pub struct EncryptedStream {
    pub noise: Mutex<TransportState>,
    pub writer: Mutex<OwnedWriteHalf>,
    pub reader: Mutex<OwnedReadHalf>,
}

impl EncryptedStream {
    pub async fn send(&self, msg: &[u8]) -> tokio::io::Result<()> {
        println!("[send] Preparing to send message: {:?}", msg);

        let mut buf = [0u8; 4096];
        let len;
        {
            println!("[send] Locking noise state for encryption");
            let mut lock = self.noise.lock().await;
            len = lock
                .write_message(msg, &mut buf)
                .map_err(|e| tokio::io::Error::new(tokio::io::ErrorKind::Other, e))?;
            println!("[send] Encrypted message length: {len}");
        }

        println!("[send] Locking writer to send encrypted data");
        let mut lock = self.writer.lock().await;
        println!("[send] Sending encrypted bytes: {:?}", &buf[..len]);
        lock.write_all(&buf[..len]).await?;
        println!("[send] Successfully sent {len} bytes");
        Ok(())
    }

    pub async fn recv(&self) -> tokio::io::Result<Vec<u8>> {
        println!("[recv] Waiting to read data from stream");

        let mut msg = [0u8; 4096];
        let n;
        {
            println!("[recv] Locking reader to fetch incoming data");
            let mut lock = self.reader.lock().await;
            n = lock.read(&mut msg).await?;
        }
        println!("[recv] Read {n} bytes: {:?}", &msg[..n]);

        let mut buf = [0u8; 4096];
        println!("[recv] Locking noise state for decryption");
        let mut lock = self.noise.lock().await;
        let len = lock
            .read_message(&msg[..n], &mut buf)
            .map_err(|e| tokio::io::Error::new(tokio::io::ErrorKind::Other, e))?;
        println!("[recv] Successfully decrypted {len} bytes");

        Ok(buf[..len].to_vec())
    }
}

pub async fn ping_client(stream: EncryptedStream) {
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
        let mut stdin_reader = BufReader::new(tokio::io::stdin());
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

pub async fn ping_server(stream: &mut EncryptedStream, addr: SocketAddr) {
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
