use bytes::{Bytes, BytesMut};
use common::EncryptedStream;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};

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

pub struct Muxer {
    inner: Arc<EncryptedStream>,
    next_stream_id: Mutex<u32>, // allocate ids (odd/even handled by caller)
    streams: Mutex<HashMap<u32, mpsc::Sender<Bytes>>>, // stream_id -> sender to per-stream handler
    incoming_tx: mpsc::Sender<(u32, String, mpsc::Receiver<Bytes>)>, // reader -> app (for new incoming streams)
    incoming_rx: Mutex<mpsc::Receiver<(u32, String, mpsc::Receiver<Bytes>)>>,
}

impl Muxer {
    /// Create the muxer. `initiator=true` => start ids at 1 (odd), else 2.
    pub fn new(inner: Arc<EncryptedStream>, initiator: bool) -> Arc<Self> {
        let start = if initiator { 1 } else { 2 };
        let (tx, rx) = mpsc::channel(32);
        Arc::new(Self {
            inner,
            next_stream_id: Mutex::new(start),
            streams: Mutex::new(HashMap::new()),
            incoming_tx: tx,
            incoming_rx: Mutex::new(rx),
        })
    }

    /// Spawn the background reader. Call this once.
    pub fn start_reader(self: &Arc<Self>) {
        let s = Arc::clone(self);
        tokio::spawn(async move {
            s.reader_loop().await;
        });
    }

    /// Reader loop: pulls frames from EncryptedStream, decodes, routes them.
    async fn reader_loop(self: Arc<Self>) {
        loop {
            let raw = match self.inner.recv().await {
                Ok(b) => b,
                Err(e) => {
                    println!("[muxer] underlying recv error: {:?}", e);
                    break;
                }
            };

            match Frame::decode(&raw) {
                Ok((frame, _consumed)) => {
                    match frame.t {
                        FrameType::Open => {
                            // payload is protocol name
                            let proto = String::from_utf8_lossy(&frame.payload).to_string();
                            // create channel the handler will read from
                            let (tx, rx) = mpsc::channel::<Bytes>(32);
                            {
                                let mut map = self.streams.lock().await;
                                map.insert(frame.stream_id, tx);
                            }
                            // notify application of incoming stream
                            let _ = self.incoming_tx.send((frame.stream_id, proto, rx)).await;
                        }
                        FrameType::Data => {
                            let maybe = {
                                let map = self.streams.lock().await;
                                map.get(&frame.stream_id).cloned()
                            };
                            if let Some(tx) = maybe {
                                // best-effort send
                                let _ = tx.send(frame.payload).await;
                            } else {
                                println!("[muxer] data for unknown stream {}", frame.stream_id);
                            }
                        }
                        FrameType::Close | FrameType::Reset => {
                            // remove stream and close channel
                            let maybe = {
                                let mut map = self.streams.lock().await;
                                map.remove(&frame.stream_id)
                            };
                            if maybe.is_some() {
                                println!("[muxer] stream {} closed/removed", frame.stream_id);
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("[muxer] frame decode error: {:?}", e);
                    // continue (you can decide to break/terminate connection)
                }
            }
        }

        println!("[muxer] reader exiting");
    }

    /// Open an outgoing stream with a protocol name.
    /// Returns (stream_id, receiver) where `receiver` yields Bytes for Data frames from peer.
    pub async fn open_stream(
        self: &Arc<Self>,
        protocol: &str,
    ) -> Result<(u32, mpsc::Receiver<Bytes>), std::io::Error> {
        // allocate id
        let id = {
            let mut lock = self.next_stream_id.lock().await;
            let id = *lock;
            *lock = id.wrapping_add(2);
            id
        };

        // create per-stream rx/tx and register tx in map so incoming DATA gets routed
        let (tx, rx) = mpsc::channel::<Bytes>(32);
        {
            let mut map = self.streams.lock().await;
            map.insert(id, tx);
        }

        // send OPEN frame with protocol name as payload
        let frame = Frame {
            t: FrameType::Open,
            stream_id: id,
            payload: Bytes::from(protocol.to_string()),
        };
        let enc = frame.encode();
        self.inner.send(&enc).await?;
        Ok((id, rx))
    }

    /// Accept next incoming stream (server side). Returns (stream_id, protocol, receiver)
    /// awaits until a remote opens a stream.
    pub async fn accept_stream(&self) -> Option<(u32, String, mpsc::Receiver<Bytes>)> {
        let mut rx = self.incoming_rx.lock().await;
        rx.recv().await
    }

    /// Send application data on stream_id
    pub async fn send_data(&self, stream_id: u32, data: &[u8]) -> Result<(), std::io::Error> {
        let frame = Frame {
            t: FrameType::Data,
            stream_id,
            payload: Bytes::copy_from_slice(data),
        };
        let enc = frame.encode();
        self.inner.send(&enc).await?;
        Ok(())
    }

    /// Close stream (notify remote and remove local state)
    pub async fn close_stream(&self, stream_id: u32) -> Result<(), std::io::Error> {
        {
            let mut map = self.streams.lock().await;
            map.remove(&stream_id);
        }
        let frame = Frame {
            t: FrameType::Close,
            stream_id,
            payload: Bytes::new(),
        };
        let enc = frame.encode();
        self.inner.send(&enc).await?;
        Ok(())
    }
}
