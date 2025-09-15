use common::EncryptedStream;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, BufReader};

pub async fn negotiate_protocol(
    stream: &mut EncryptedStream,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) {
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
        } else {
            eprintln!("[negotiate_protocol] ❌ Unimplemented protocol");
        }
    } else {
        eprintln!("[negotiate_protocol] Unsupported negotiation protocol: {other}", other = proto);
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
            supported_protocols.get("protocol")
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

        if let Some(p) = supported_protocols.get("protocol") {
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
