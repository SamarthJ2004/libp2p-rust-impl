use snow::TransportState;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
};

#[derive(Debug)]
pub struct EncryptedStream {
    pub noise: TransportState,
    pub writer: OwnedWriteHalf,
    pub reader: OwnedReadHalf,
}

impl EncryptedStream {
    pub async fn send(&mut self, msg: &[u8]) -> tokio::io::Result<()> {
        let mut buf = [0u8; 65535];
        let len = self.noise.write_message(msg, &mut buf).unwrap();
        self.writer.write_all(&buf[..len]).await
    }

    pub async fn recv(&mut self) -> tokio::io::Result<Vec<u8>> {
        let mut msg = [0u8; 65535];
        let n = self.reader.read(&mut msg).await?;
        let mut buf = [0u8; 65535];

        let len = self.noise.read_message(&msg[..n], &mut buf).unwrap();
        Ok(buf[..len].to_vec())
    }
}
