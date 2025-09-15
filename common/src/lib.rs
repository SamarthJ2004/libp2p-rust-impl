use snow::TransportState;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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
