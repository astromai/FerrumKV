//! Pruebas del límite de tamaño de comandos en `Connection::read_line`.
//!
//! Cada prueba crea un par de sockets TCP reales: el lado "servidor" se
//! envuelve en `Connection` y el lado "cliente" envía bytes crudos.

use std::io::ErrorKind;
use std::time::Duration;

use ferrum_kv::network::Connection;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const MAX: usize = Connection::MAX_LINE_LENGTH;

/// Devuelve (conexión del servidor, socket del cliente).
async fn pair() -> (Connection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (Connection::new(server), client)
}

/// Escribe en segundo plano para no bloquearse si el buffer del socket se llena.
fn send(mut client: TcpStream, data: Vec<u8>) -> tokio::task::JoinHandle<TcpStream> {
    tokio::spawn(async move {
        // Si el servidor corta la lectura, la escritura puede fallar: es esperado.
        let _ = client.write_all(&data).await;
        client
    })
}

async fn read_owned(conn: &mut Connection) -> std::io::Result<Option<String>> {
    conn.read_line().await.map(|o| o.map(str::to_owned))
}

#[tokio::test]
async fn comando_normal_lf() {
    let (mut conn, client) = pair().await;
    let _c = send(client, b"PING hola\n".to_vec()).await.unwrap();
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("PING hola")
    );
}

#[tokio::test]
async fn comando_normal_crlf() {
    let (mut conn, client) = pair().await;
    let _c = send(client, b"PING hola\r\n".to_vec()).await.unwrap();
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("PING hola")
    );
}

#[tokio::test]
async fn varias_lineas_en_un_solo_envio() {
    let (mut conn, client) = pair().await;
    let _c = send(client, b"PING\r\nDBSIZE\nQUIT\r\n".to_vec())
        .await
        .unwrap();
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("PING")
    );
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("DBSIZE")
    );
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("QUIT")
    );
}

#[tokio::test]
async fn linea_menor_al_limite() {
    let (mut conn, client) = pair().await;
    let line = "a".repeat(MAX - 1);
    let _c = send(client, format!("{line}\n").into_bytes())
        .await
        .unwrap();
    assert_eq!(read_owned(&mut conn).await.unwrap(), Some(line));
}

#[tokio::test]
async fn linea_exactamente_en_el_limite_lf() {
    let (mut conn, client) = pair().await;
    let line = "a".repeat(MAX);
    let _c = send(client, format!("{line}\n").into_bytes())
        .await
        .unwrap();
    assert_eq!(read_owned(&mut conn).await.unwrap(), Some(line));
}

#[tokio::test]
async fn linea_exactamente_en_el_limite_crlf() {
    let (mut conn, client) = pair().await;
    let line = "a".repeat(MAX);
    let _c = send(client, format!("{line}\r\n").into_bytes())
        .await
        .unwrap();
    assert_eq!(read_owned(&mut conn).await.unwrap(), Some(line));
}

#[tokio::test]
async fn linea_sobre_el_limite_lf() {
    let (mut conn, client) = pair().await;
    let _c = send(client, format!("{}\n", "a".repeat(MAX + 1)).into_bytes());
    let err = read_owned(&mut conn).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn linea_sobre_el_limite_crlf() {
    let (mut conn, client) = pair().await;
    let _c = send(client, format!("{}\r\n", "a".repeat(MAX + 1)).into_bytes());
    let err = read_owned(&mut conn).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

/// Un `\r` de contenido + `\r\n` no debe colarse como byte extra permitido.
#[tokio::test]
async fn limite_con_retornos_de_carro_extra() {
    let (mut conn, client) = pair().await;
    let line = "a".repeat(MAX);
    let _c = send(client, format!("{line}\r\r\n").into_bytes())
        .await
        .unwrap();
    assert_eq!(
        read_owned(&mut conn).await.unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

/// Un cliente que envía bytes sin salto de línea y NO cierra la conexión
/// debe recibir error en cuanto supera el límite, sin acumular memoria.
#[tokio::test]
async fn flujo_sin_terminador_falla_sin_esperar_eof() {
    let (mut conn, client) = pair().await;
    // El cliente queda vivo dentro de la tarea (no cierra el socket).
    let _c = send(client, vec![b'a'; MAX * 4]);

    let result = timeout(Duration::from_secs(5), read_owned(&mut conn))
        .await
        .expect("read_line debió fallar sin esperar un salto de línea");
    assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn eof_sin_datos_devuelve_none() {
    let (mut conn, client) = pair().await;
    drop(client);
    assert_eq!(read_owned(&mut conn).await.unwrap(), None);
}

#[tokio::test]
async fn ultima_linea_sin_terminador_antes_de_eof() {
    let (mut conn, mut client) = pair().await;
    client.write_all(b"PING").await.unwrap();
    drop(client);
    assert_eq!(
        read_owned(&mut conn).await.unwrap().as_deref(),
        Some("PING")
    );
    assert_eq!(read_owned(&mut conn).await.unwrap(), None);
}

#[tokio::test]
async fn utf8_invalido_es_invalid_data() {
    let (mut conn, client) = pair().await;
    let _c = send(client, vec![0xff, 0xfe, b'\n']).await.unwrap();
    assert_eq!(
        read_owned(&mut conn).await.unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[tokio::test]
async fn write_line_agrega_crlf() {
    let (mut conn, client) = pair().await;
    conn.write_line("+PONG").await.unwrap();
    let mut reader = BufReader::new(client);
    let mut got = String::new();
    reader.read_line(&mut got).await.unwrap();
    assert_eq!(got, "+PONG\r\n");
}

/// De punta a punta: el servidor real cierra la conexión ante un comando
/// demasiado largo y sigue atendiendo a otros clientes con normalidad.
#[tokio::test]
async fn servidor_cierra_conexion_abusiva_y_sigue_funcionando() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ferrum_kv::server::run(
        listener,
        std::future::pending::<()>(),
    ));

    // Cliente abusivo: envía muchos bytes sin salto de línea.
    let abusivo = TcpStream::connect(addr).await.unwrap();
    let (mut lectura, mut escritura) = abusivo.into_split();
    tokio::spawn(async move {
        let _ = escritura.write_all(&vec![b'a'; MAX * 4]).await;
        // Mantiene el socket abierto: no envía EOF.
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let mut resto = Vec::new();
    let n = timeout(
        Duration::from_secs(5),
        tokio::io::AsyncReadExt::read_to_end(&mut lectura, &mut resto),
    )
    .await
    .expect("el servidor debió cerrar la conexión abusiva");
    // El servidor cierra sin responder (o con reset): nunca envía datos.
    assert!(n.map(|n| n == 0).unwrap_or(true));

    // Un cliente normal sigue siendo atendido.
    let mut normal = BufReader::new(TcpStream::connect(addr).await.unwrap());
    normal
        .get_mut()
        .write_all(b"PING\r\n".as_slice())
        .await
        .unwrap();
    let mut resp = String::new();
    normal.read_line(&mut resp).await.unwrap();
    assert_eq!(resp, "+PONG\r\n");
}
