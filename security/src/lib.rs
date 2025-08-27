use std::collections::HashMap;

use snow::{Builder, Keypair, TransportState};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
};

pub async fn negotiate_security_protocol(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> TransportState {
    let _ = writer.write_all(b"/multistream/1.0.0\n").await;
    let mut line = String::new();
    let _ = reader.read_line(&mut line).await;
    println!("Received negotitation protocol: {}", line.trim());
    match line.trim() {
        "/multistream/1.0.0" => loop {
            if let Some(transport) =
                negotiate_security(reader, writer, is_initiator, &supported_protocols)
                    .await
                    .unwrap()
            {
                return transport;
            } else {
                eprintln!("Unimplemented Protocol");
                continue;
            }
        },
        _ => {
            eprintln!("Unsupported Negotiation Protocol");
            std::process::exit(1);
        }
    };
}

async fn negotiate_security(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> tokio::io::Result<Option<TransportState>> {
    let private_key = generate_static_keypair().private;
    if let Some(protocols) = supported_protocols.get("security") {
        if is_initiator {
            let mut input = String::new();
            let mut stdin_reader = BufReader::new(tokio::io::stdin());

            println!("Enter protocol to propose: {:?}", protocols);
            stdin_reader.read_line(&mut input).await?;
            let proto = input.trim();

            let _ = writer.write_all(format!("{proto}\n").as_bytes()).await;

            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;

            if line.trim() == proto {
                if proto == "/noise/xx" {
                    println!("Server agreed on {proto}");
                    return Ok(Some(
                        perform_noise_initiator_handshake(reader.get_mut(), writer, &private_key)
                            .await
                            .unwrap(),
                    ));
                } else {
                    return Ok(None);
                }
            } else {
                eprintln!("Rejected Protocol");
                std::process::exit(1);
            }
        } else {
            let mut line = String::new();
            reader.read_line(&mut line).await?;

            let proto = line.trim();

            if protocols.contains(&proto) {
                writer.write_all(format!("{proto}\n").as_bytes()).await;
                let transport =
                    perform_noise_responder_handshake(reader.get_mut(), writer, &private_key).await;

                println!("Client agreed on {proto}");
                return Ok(Some(transport.unwrap()));
            } else {
                writer.write_all(b"na\n").await?;
                return Ok(None);
            }
        }
    } else {
        panic!("No security protocols found");
    }
}

fn generate_static_keypair() -> Keypair {
    let builder: snow::Builder<'_> =
        snow::Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap());

    builder.generate_keypair().unwrap()
}

fn noise_builder(private_key: &[u8]) -> Builder<'_> {
    Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap())
        .local_private_key(private_key)
        .unwrap()
}

pub async fn perform_noise_initiator_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    local_private: &[u8],
) -> tokio::io::Result<TransportState> {
    println!("Entered initiator noise handshake");
    let builder = noise_builder(local_private);
    let mut noise = builder.build_initiator().unwrap();

    let mut buf = [0u8; 65535];

    let len = noise.write_message(&[], &mut buf).unwrap_or(0);
    writer.write_all(&buf[..len]).await?;

    let mut msg = vec![0u8; 1024];
    let n = reader.read(&mut msg).await.unwrap();
    noise.read_message(&msg[..n], &mut buf).unwrap();

    let len = noise.write_message(&[], &mut buf).unwrap();
    writer.write_all(&buf[..len]).await?;

    Ok(noise.into_transport_mode().unwrap())
}

pub async fn perform_noise_responder_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    local_private: &[u8],
) -> tokio::io::Result<TransportState> {
    println!("Entered responder noise handshake");
    let builder = noise_builder(local_private);
    let mut noise = builder.build_responder().unwrap();

    let mut buf = [0u8; 65535];

    let mut msg = vec![0u8; 1024];
    let n = reader.read(&mut msg).await.unwrap();
    noise.read_message(&msg[..n], &mut buf).unwrap();

    let len = noise.write_message(&[], &mut buf).unwrap();
    writer.write_all(&buf[..len]).await?;

    let n = reader.read(&mut msg).await.unwrap();
    noise.read_message(&msg[..n], &mut buf).unwrap();

    println!("Done responder handshake");
    Ok(noise.into_transport_mode().unwrap())
}
