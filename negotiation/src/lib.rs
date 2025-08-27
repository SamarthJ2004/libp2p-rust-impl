use common::EncryptedStream;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, BufReader};

pub async fn negotiate_protocol(
    stream: &mut EncryptedStream,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) {
    let _ = stream.send(b"/multistream/1.0.0\n").await;
    let response = stream.recv().await.unwrap();
    let proto = String::from_utf8_lossy(&response);
    println!("Received negotitation protocol: {}", proto);

    match proto.trim() {
        "/multistream/1.0.0" => loop {
            if let Some(transport) = negotiate(stream, is_initiator, &supported_protocols)
                .await
                .unwrap()
            {
                println!("Agreed on {transport}");
                break;
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
async fn negotiate(
    stream: &mut EncryptedStream,
    is_initiator: bool,
    supported_protocols: &HashMap<&'static str, Vec<&'static str>>,
) -> tokio::io::Result<Option<String>> {
    if is_initiator {
        let mut input = String::new();
        let mut stdin_reader = BufReader::new(tokio::io::stdin());
        println!(
            "Enter protocol to propose: {:?}",
            supported_protocols.get("protocol")
        );

        stdin_reader.read_line(&mut input).await?;
        let proto = input.trim();

        stream.send(format!("{proto}\n").as_bytes()).await?;

        let response = stream.recv().await.unwrap();

        let line = String::from_utf8_lossy(&response);
        println!("Received reponse: {}", line);

        if line.trim() == proto {
            println!("Negotiated Protocol: {proto}");
            Ok(Some(proto.to_string()))
        } else {
            println!("Protocol rejected: {}", line.trim());
            Ok(None)
        }
    } else {
        let response = stream.recv().await.unwrap();
        let line = String::from_utf8_lossy(&response);

        let proposal = line.trim();

        if let Some(p) = supported_protocols.get("protocol") {
            if p.contains(&proposal) {
                stream.send(format!("{}\n", proposal).as_bytes()).await?;
                Ok(Some(proposal.to_string()))
            } else {
                stream.send(b"na\n").await?;
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}
