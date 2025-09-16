use bytes::{Bytes, BytesMut};
use common::EncryptedStream;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Open = 1,
    Data = 2,
    Close = 3,
    Reset = 4,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub t: FrameType,
    pub stream_id: u32,
    pub payload: Bytes,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum FrameDecodeError {
    #[error("buffer too short")]
    TooShort,
    #[error("unknown frame type {0}")]
    UnknownType(u8),
    #[error("declared payload too large: {0}")]
    TooLarge(u32),
    #[error("payload length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch { declared: usize, actual: usize },
}

impl Frame {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.extend_from_slice(&self.stream_id.to_le_bytes());
        buf.extend_from_slice(&(self.t as u8).to_le_bytes());
        buf.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.payload);

        buf.freeze()
    }

    pub fn decode(buf: &[u8]) -> Result<(Frame, usize), FrameDecodeError> {
        if buf.len() < 4 + 1 + 4 {
            return Err(FrameDecodeError::TooShort);
        }

        let stream_id = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let t_raw = buf[4];
        let len = u32::from_le_bytes(buf[5..9].try_into().unwrap()) as usize;

        if buf.len() < 9 + len {
            return Err(FrameDecodeError::LengthMismatch {
                declared: len,
                actual: buf.len() - 9,
            });
        }

        let t = match t_raw {
            1 => FrameType::Open,
            2 => FrameType::Data,
            3 => FrameType::Close,
            4 => FrameType::Reset,
            other => return Err(FrameDecodeError::UnknownType(other)),
        };

        let payload = Bytes::copy_from_slice(&buf[9..9 + len]);

        Ok((
            Frame {
                t,
                stream_id,
                payload,
            },
            9 + len,
        ))
    }
}

pub async fn negotiate_multiplexing_protocol(
    stream: &mut EncryptedStream,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> tokio::io::Result<String> {
    if is_initiator {
        println!("[negotiate_protocol] -> Sending /multistream/1.0.0");
        let _ = stream.send(b"/multistream/1.0.0\n").await;
    }

    let response = stream.recv().await.unwrap();
    let proto = String::from_utf8_lossy(&response);
    println!(
        "[negotiate_protocol] <- Received negotiation protocol: {}",
        proto
    );

    if proto.trim() == "/multistream/1.0.0" {
        if !is_initiator {
            println!("[negotiate_protocol] -> Sending /multistream/1.0.0");
            let _ = stream.send(b"/multistream/1.0.0\n").await;
        }
        println!("[negotiate_protocol] Entering subprotocol negotiation");
        if let Some(transport) = negotiate(stream, is_initiator, &supported_protocols)
            .await
            .unwrap()
        {
            println!("[negotiate_protocol] ✅ Agreed on protocol: {transport}");
            return Ok(transport);
        } else {
            eprintln!("[negotiate_protocol] ❌ Unimplemented protocol");
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported negotiation protocol: {}", proto),
            ));
        }
    } else {
        eprintln!(
            "[negotiate_protocol] Unsupported negotiation protocol: {other}",
            other = proto
        );
        std::process::exit(1);
    }
}

async fn negotiate(
    stream: &mut EncryptedStream,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> tokio::io::Result<Option<String>> {
    println!("[negotiate] Started negotiation, initiator={is_initiator}");

    if is_initiator {
        let mut input = String::new();
        let mut stdin_reader = BufReader::new(tokio::io::stdin());
        println!(
            "[negotiate][initiator] Available protocols: {:?}",
            supported_protocols.get("multiplexing")
        );

        stdin_reader.read_line(&mut input).await?;
        let proto = input.trim();
        println!("[negotiate][initiator] Proposing protocol: {proto}");

        stream.send(format!("{proto}\n").as_bytes()).await?;

        let response = stream.recv().await.unwrap();
        let line = String::from_utf8_lossy(&response);
        println!("[negotiate][initiator] <- Received response: {}", line);

        if line.trim() == proto {
            println!("[negotiate][initiator] ✅ Negotiated protocol: {proto}");
            Ok(Some(proto.to_string()))
        } else {
            println!(
                "[negotiate][initiator] ❌ Protocol rejected by responder: {}",
                line.trim()
            );
            Ok(None)
        }
    } else {
        println!("[negotiate][responder] Waiting for initiator proposal");
        let response = stream.recv().await.unwrap();
        let line = String::from_utf8_lossy(&response);
        let proposal = line.trim();
        println!("[negotiate][responder] <- Received proposal: {proposal}");

        if let Some(p) = supported_protocols.get("multiplexing") {
            if p.contains(&proposal) {
                println!("[negotiate][responder] ✅ Accepting proposal: {proposal}");
                stream.send(format!("{}\n", proposal).as_bytes()).await?;
                Ok(Some(proposal.to_string()))
            } else {
                eprintln!(
                    "[negotiate][responder] ❌ Unsupported proposal: {proposal}, replying 'na'"
                );
                stream.send(b"na\n").await?;
                Ok(None)
            }
        } else {
            eprintln!("[negotiate][responder] ⚠️ No protocols found in supported_protocols");
            Ok(None)
        }
    }
}
