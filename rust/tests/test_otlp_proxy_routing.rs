//! OTLP loopback delivery must be independent of host proxy settings.
//! This target runs in its own process because it changes the proxy environment.

use std::time::Duration;

use apcore::observability::exporters::OTLPExporter;
use apcore::observability::span::{Span, SpanExporter};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

fn set_proxy(url: &str) {
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(key, url);
    }
    for key in ["NO_PROXY", "no_proxy", "REQUEST_METHOD"] {
        std::env::remove_var(key);
    }
}

async fn receiver() -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind receiver");
    let port = listener.local_addr().expect("receiver address").port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept request");
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .await
            .expect("request line");
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).await.expect("header line");
            assert_ne!(read, 0, "request ended before headers were complete");
            if line == "\r\n" {
                break;
            }
            if let Some(length) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = length.trim().parse().expect("content length");
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).await.expect("request body");
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("response");
        request_line
    });
    (port, server)
}

async fn export_to(endpoint: String, server: JoinHandle<String>) -> String {
    // The remote-proxy case also initializes the platform's TLS and proxy stack.
    let destination = endpoint.clone();
    tokio::time::timeout(Duration::from_secs(30), async {
        OTLPExporter::new(endpoint)
            .export(&Span::new("proxy-routing", "trace-proxy-routing"))
            .await
            .expect("export span");
        server.await.expect("receiver completed")
    })
    .await
    .unwrap_or_else(|error| panic!("OTLP delivery to {destination} timed out: {error}"))
}

#[tokio::test]
async fn test_loopback_bypasses_proxy_and_remote_endpoint_uses_proxy() {
    // An unusable proxy makes unintended proxy routing fail immediately.
    set_proxy("http://127.0.0.1:0");
    for host in ["127.0.0.1", "localhost", "[::ffff:127.0.0.1]"] {
        let (port, server) = receiver().await;
        let request = export_to(format!("http://{host}:{port}/v1/traces"), server).await;
        assert_eq!(request, "POST /v1/traces HTTP/1.1\r\n");
    }

    // A local proxy handles an unresolvable remote hostname without external I/O.
    let (port, server) = receiver().await;
    set_proxy(&format!("http://127.0.0.1:{port}"));
    let request = export_to(
        "http://collector.invalid:4318/v1/traces".to_string(),
        server,
    )
    .await;
    assert_eq!(
        request,
        "POST http://collector.invalid:4318/v1/traces HTTP/1.1\r\n"
    );
}
