use ferrum_kv::server;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Levanta el servidor en un puerto libre y devuelve su dirección.
async fn start_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(listener, std::future::pending::<()>()));
    addr
}

async fn send(stream: &mut BufReader<TcpStream>, cmd: &str) -> String {
    stream.get_mut().write_all(format!("{cmd}\n").as_bytes()).await.unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    line.trim_end().to_string()
}

#[tokio::test]
async fn ping_responde_pong() {
    let addr = start_server().await;
    let mut c = BufReader::new(TcpStream::connect(addr).await.unwrap());
    assert_eq!(send(&mut c, "PING").await, "+PONG");
    assert_eq!(send(&mut c, "ping hola").await, "hola");
    assert!(send(&mut c, "FOO").await.starts_with("-ERR"));
}

#[tokio::test]
async fn muchos_clientes_simultaneos() {
    let addr = start_server().await;

    // 200 clientes conectados al mismo tiempo, cada uno envía 50 comandos.
    let mut handles = Vec::new();
    for i in 0..200 {
        handles.push(tokio::spawn(async move {
            let mut c = BufReader::new(TcpStream::connect(addr).await.unwrap());
            for j in 0..50 {
                let msg = format!("c{i}-m{j}");
                assert_eq!(send(&mut c, &format!("PING {msg}")).await, msg);
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}
