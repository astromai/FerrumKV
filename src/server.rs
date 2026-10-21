use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::network::Connection;
use crate::storage::Db;
use crate::MAX_CONNECTIONS;

/// Bucle principal: acepta clientes y lanza una tarea liviana por cada uno.
///
/// `shutdown` es un future que, al completarse (p. ej. Ctrl+C), detiene el
/// servidor ordenadamente. Recibirlo como parámetro facilita los tests.
pub async fn run(listener: TcpListener, shutdown: impl Future) {
    let db = Db::new();
    // Limita los clientes simultáneos para no agotar memoria/descriptores.
    let limit = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    tokio::select! {
        _ = accept_loop(listener, db, limit) => {}
        _ = shutdown => {
            println!("Apagando servidor...");
        }
    }
}

async fn accept_loop(listener: TcpListener, db: Db, limit: Arc<Semaphore>) {
    loop {
        // Espera un cupo libre ANTES de aceptar al siguiente cliente.
        let permit = limit.clone().acquire_owned().await.unwrap();

        let (socket, addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("Error al aceptar conexión: {e}");
                continue;
            }
        };

        // Cada clon de `Db` es solo un `Arc` más: barato y comparte el mismo mapa.
        let db = db.clone();

        tokio::spawn(async move {
            println!("[+] Cliente conectado: {addr}");
            if let Err(e) = handle_client(socket, addr, db).await {
                eprintln!("[!] Error con {addr}: {e}");
            }
            println!("[-] Cliente desconectado: {addr}");
            drop(permit); // libera el cupo al terminar
        });
    }
}

/// Atiende a UN cliente: lee comandos línea a línea y responde.
async fn handle_client(socket: TcpStream, _addr: SocketAddr, db: Db) -> std::io::Result<()> {
    let mut conn = Connection::new(socket);

    while let Some(line) = conn.read_line().await? {
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else { continue }; // línea vacía

        // Por ahora: comandos de prueba de conectividad.
        // SET/GET/DEL/INCR se agregan en el siguiente paso.
        let response = match cmd.to_ascii_uppercase().as_str() {
            "PING" => match parts.next() {
                Some(msg) => msg.to_string(),
                None => "+PONG".to_string(),
            },
            "DBSIZE" => format!(":{}", db.len()),
            "QUIT" => {
                conn.write_line("+OK").await?;
                return Ok(());
            }
            other => format!("-ERR unknown command '{other}'"),
        };

        conn.write_line(&response).await?;
    }
    Ok(())
}
