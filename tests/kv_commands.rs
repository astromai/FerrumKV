use ferrum_kv::server;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

async fn start_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(listener, std::future::pending::<()>()));
    addr
}

async fn send(client: &mut BufReader<TcpStream>, command: &str) -> String {
    client
        .get_mut()
        .write_all(format!("{command}\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    assert!(client.read_line(&mut response).await.unwrap() > 0);
    response.trim_end_matches(['\r', '\n']).to_string()
}

#[tokio::test]
async fn set_get_del_and_dbsize_share_state_between_clients() {
    let addr = start_server().await;
    let mut writer = BufReader::new(TcpStream::connect(addr).await.unwrap());
    let mut reader = BufReader::new(TcpStream::connect(addr).await.unwrap());

    assert_eq!(send(&mut reader, "GET missing").await, "$-1");
    assert_eq!(send(&mut writer, "DBSIZE").await, ":0");
    assert_eq!(send(&mut writer, "set greeting hola  mundo").await, "+OK");
    assert_eq!(send(&mut reader, "GET greeting").await, "+hola  mundo");
    assert_eq!(send(&mut reader, "DBSIZE").await, ":1");
    assert_eq!(send(&mut writer, "SET greeting adios").await, "+OK");
    assert_eq!(send(&mut reader, "GET greeting").await, "+adios");
    assert_eq!(send(&mut reader, "DBSIZE").await, ":1");
    assert_eq!(send(&mut reader, "del greeting").await, ":1");
    assert_eq!(send(&mut writer, "DEL greeting").await, ":0");
    assert_eq!(send(&mut writer, "GET greeting").await, "$-1");
    assert_eq!(send(&mut reader, "DBSIZE").await, ":0");
}

#[tokio::test]
async fn incr_handles_missing_numeric_invalid_and_overflow_values() {
    let addr = start_server().await;
    let mut client = BufReader::new(TcpStream::connect(addr).await.unwrap());

    assert_eq!(send(&mut client, "INCR counter").await, ":1");
    assert_eq!(send(&mut client, "incr counter").await, ":2");
    assert_eq!(send(&mut client, "SET counter -2").await, "+OK");
    assert_eq!(send(&mut client, "INCR counter").await, ":-1");
    assert_eq!(send(&mut client, "SET counter texto").await, "+OK");
    assert_eq!(
        send(&mut client, "INCR counter").await,
        "-ERR value is not an integer or out of range"
    );
    assert_eq!(send(&mut client, "GET counter").await, "+texto");
    assert_eq!(send(&mut client, "SET counter 9223372036854775807").await, "+OK");
    assert_eq!(
        send(&mut client, "INCR counter").await,
        "-ERR value is not an integer or out of range"
    );
    assert_eq!(
        send(&mut client, "GET counter").await,
        "+9223372036854775807"
    );
}

#[tokio::test]
async fn errors_and_existing_commands_keep_connection_usable() {
    let addr = start_server().await;
    let mut client = BufReader::new(TcpStream::connect(addr).await.unwrap());

    for command in [
        "SET key",
        "SET key   ",
        "GET",
        "GET a b",
        "DEL",
        "DEL a b",
        "INCR",
        "INCR a b",
        "DBSIZE extra",
        "QUIT extra",
    ] {
        assert!(
            send(&mut client, command).await.starts_with("-ERR"),
            "{command}"
        );
    }
    assert!(send(&mut client, "UNKNOWN").await.starts_with("-ERR"));
    assert_eq!(send(&mut client, "PING").await, "+PONG");
    assert_eq!(send(&mut client, "ping hola").await, "hola");
    assert_eq!(send(&mut client, "DBSIZE").await, ":0");
    assert_eq!(send(&mut client, "QUIT").await, "+OK");

    let mut response = String::new();
    assert_eq!(client.read_line(&mut response).await.unwrap(), 0);
}

#[tokio::test]
async fn concurrent_increments_do_not_lose_updates() {
    let addr = start_server().await;
    let mut tasks = Vec::new();

    for _ in 0..24 {
        tasks.push(tokio::spawn(async move {
            let mut client = BufReader::new(TcpStream::connect(addr).await.unwrap());
            for _ in 0..40 {
                assert!(send(&mut client, "INCR counter").await.starts_with(':'));
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let mut client = BufReader::new(TcpStream::connect(addr).await.unwrap());
    assert_eq!(send(&mut client, "GET counter").await, "+960");
    assert_eq!(send(&mut client, "DBSIZE").await, ":1");
}
