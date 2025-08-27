use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
};

const SUPPORTED_PROTOCOLS: [&str; 1] = ["/ping/1.0.0"];

pub async fn negotiate_protocol(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    is_initiator: bool,
) {
    let _ = writer.write_all(b"/multistream/1.0.0\n").await;
    let mut line = String::new();
    reader.read_line(&mut line).await;
    println!("Received negotitation protocol: {}", line.trim());
    match line.trim() {
        "/multistream/1.0.0" => loop {
            if let Some(proto) = negotiate(reader, writer, is_initiator).await.unwrap() {
                println!("Client agreed on the protocol : {proto}");
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
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    is_initiator: bool,
) -> tokio::io::Result<Option<String>> {
    if is_initiator {
        let mut input = String::new();
        let mut stdin_reader = BufReader::new(tokio::io::stdin());
        println!("Enter protocol to propose: {:?}", SUPPORTED_PROTOCOLS);

        stdin_reader.read_line(&mut input).await;
        let proto = input.trim();

        let _ = writer.write_all(format!("{proto}\n").as_bytes()).await;

        let mut line = String::new();
        let _ = reader.read_line(&mut line).await;

        if line.trim() == proto {
            println!("Negotiated Protocol: {proto}");
            Ok(Some(proto.to_string()))
        } else {
            println!("Protocol rejected: {}", line.trim());
            Ok(None)
        }
    } else {
        let mut line = String::new();
        let _ = reader.read_line(&mut line).await;

        let proposal = line.trim();

        if SUPPORTED_PROTOCOLS.contains(&proposal) {
            let _ = writer.write_all(format!("{}\n", proposal).as_bytes()).await;
            Ok(Some(proposal.to_string()))
        } else {
            let _ = writer.write_all(b"na\n").await;
            Ok(None)
        }
    }
}
