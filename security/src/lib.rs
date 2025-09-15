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
    if is_initiator {
        println!("[negotiate_security_protocol] -> Sending /multistream/1.0.0");
        let _ = writer.write_all(b"/multistream/1.0.0\n").await;
    }

    let mut line = String::new();
    let _ = reader.read_line(&mut line).await;
    println!(
        "[negotiate_security_protocol] <- Received negotiation protocol: {}",
        line.trim()
    );

    if line.trim() == "/multistream/1.0.0" {
        if !is_initiator {
            println!("[negotiate_security_protocol] -> Sending /multistream/1.0.0");
            let _ = writer.write_all(b"/multistream/1.0.0\n").await;
        }

        println!("[negotiate_security_protocol] Entering security negotiation");
        if let Some(transport) =
            negotiate_security(reader, writer, is_initiator, &supported_protocols)
                .await
                .unwrap()
        {
            println!("[negotiate_security_protocol] Security transport established");
            return transport;
        } else {
            eprintln!("[negotiate_security_protocol] Unimplemented protocol");
            std::process::exit(1);
        }
    } else {
        eprintln!(
            "[negotiate_security_protocol] Unsupported negotiation protocol: {}",
            line.trim()
        );
        std::process::exit(1);
    }
}

async fn negotiate_security(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> tokio::io::Result<Option<TransportState>> {
    println!("[negotiate_security] Starting security negotiation, initiator={is_initiator}");
    let private_key = generate_static_keypair().private;

    if let Some(protocols) = supported_protocols.get("security") {
        println!("[negotiate_security] Supported protocols: {:?}", protocols);

        if is_initiator {
            let mut input = String::new();
            let mut stdin_reader = BufReader::new(tokio::io::stdin());

            println!(
                "[negotiate_security] Enter protocol to propose: {:?}",
                protocols
            );
            stdin_reader.read_line(&mut input).await?;
            let proto = input.trim();

            println!("[negotiate_security] Proposing protocol: {proto}");
            let _ = writer.write_all(format!("{proto}\n").as_bytes()).await;

            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;
            println!("[negotiate_security] Server responded: {}", line.trim());

            if line.trim() == proto {
                if proto == "/noise/xx" {
                    println!(
                        "[negotiate_security] Protocol {proto} accepted, running initiator handshake"
                    );
                    return Ok(Some(
                        perform_noise_initiator_handshake(reader.get_mut(), writer, &private_key)
                            .await
                            .unwrap(),
                    ));
                } else {
                    println!("[negotiate_security] Protocol {proto} not implemented yet");
                    return Ok(None);
                }
            } else {
                eprintln!("[negotiate_security] Server rejected protocol");
                std::process::exit(1);
            }
        } else {
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            let proto = line.trim();
            println!("[negotiate_security] Client proposed protocol: {proto}");

            if protocols.contains(&proto) {
                println!("[negotiate_security] Accepting protocol: {proto}");
                let _ = writer.write_all(format!("{proto}\n").as_bytes()).await;
                let transport =
                    perform_noise_responder_handshake(reader.get_mut(), writer, &private_key).await;

                println!("[negotiate_security] Completed responder handshake with {proto}");
                return Ok(Some(transport.unwrap()));
            } else {
                eprintln!("[negotiate_security] Unsupported protocol {proto}, sending 'na'");
                writer.write_all(b"na\n").await?;
                return Ok(None);
            }
        }
    } else {
        panic!("[negotiate_security] No security protocols found in supported_protocols");
    }
}

fn generate_static_keypair() -> Keypair {
    println!("[generate_static_keypair] Generating static Noise keypair");
    let builder: snow::Builder<'_> =
        snow::Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap());
    builder.generate_keypair().unwrap()
}

fn noise_builder(private_key: &[u8]) -> Builder<'_> {
    println!("[noise_builder] Building Noise XX state");
    Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap())
        .local_private_key(private_key)
        .unwrap()
}

pub async fn perform_noise_initiator_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    local_private: &[u8],
) -> tokio::io::Result<TransportState> {
    println!("[initiator_handshake] Entered initiator Noise handshake");
    let builder = noise_builder(local_private);
    let mut noise = builder.build_initiator().unwrap();

    let mut buf = [0u8; 65535];

    let len = noise.write_message(&[], &mut buf).unwrap_or(0);
    println!(
        "[initiator_handshake] -> Sending first handshake message ({} bytes)",
        len
    );
    writer.write_all(&buf[..len]).await?;

    let mut msg = vec![0u8; 1024];
    let n = reader.read(&mut msg).await.unwrap();
    println!("[initiator_handshake] <- Received {} bytes", n);
    noise.read_message(&msg[..n], &mut buf).unwrap();

    let len = noise.write_message(&[], &mut buf).unwrap();
    println!(
        "[initiator_handshake] -> Sending final handshake message ({} bytes)",
        len
    );
    writer.write_all(&buf[..len]).await?;

    println!("[initiator_handshake] Handshake complete, entering transport mode");
    Ok(noise.into_transport_mode().unwrap())
}

pub async fn perform_noise_responder_handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    local_private: &[u8],
) -> tokio::io::Result<TransportState> {
    println!("[responder_handshake] Entered responder Noise handshake");
    let builder = noise_builder(local_private);
    let mut noise = builder.build_responder().unwrap();

    let mut buf = [0u8; 65535];
    let mut msg = vec![0u8; 1024];

    let n = reader.read(&mut msg).await.unwrap();
    println!(
        "[responder_handshake] <- Received first message ({} bytes)",
        n
    );
    noise.read_message(&msg[..n], &mut buf).unwrap();

    let len = noise.write_message(&[], &mut buf).unwrap();
    println!(
        "[responder_handshake] -> Sending response message ({} bytes)",
        len
    );
    writer.write_all(&buf[..len]).await?;

    let n = reader.read(&mut msg).await.unwrap();
    println!(
        "[responder_handshake] <- Received final message ({} bytes)",
        n
    );
    noise.read_message(&msg[..n], &mut buf).unwrap();

    println!("[responder_handshake] Handshake complete, entering transport mode");
    Ok(noise.into_transport_mode().unwrap())
}
