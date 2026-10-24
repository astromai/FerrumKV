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
        let line = line.trim_start();
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else { continue };
        let args = &line[cmd.len()..];

        let response = match cmd.to_ascii_uppercase().as_str() {
            "PING" if args.trim().is_empty() => "+PONG".to_string(),
            "PING" => args.trim().to_string(),
            "DBSIZE" if args.trim().is_empty() => format!(":{}", db.len()),
            "DBSIZE" => arity_error("DBSIZE"),
            "QUIT" if args.trim().is_empty() => {
                conn.write_line("+OK").await?;
                return Ok(());
            }
            "QUIT" => arity_error("QUIT"),
            "SET" => match set_args(args) {
                Some((key, value)) => {
                    db.set(key, value);
                    "+OK".to_string()
                }
                None => arity_error("SET"),
            },
            "GET" => match key_arg(args) {
                Some(key) => db
                    .get(key)
                    .map_or_else(|| "$-1".to_string(), |value| format!("+{value}")),
                None => arity_error("GET"),
            },
            "DEL" => match key_arg(args) {
                Some(key) => format!(":{}", u8::from(db.del(key))),
                None => arity_error("DEL"),
            },
            "INCR" => match key_arg(args) {
                Some(key) => match db.incr(key) {
                    Ok(value) => format!(":{value}"),
                    Err(message) => format!("-ERR {message}"),
                },
                None => arity_error("INCR"),
            },
            other => format!("-ERR unknown command '{other}'"),
        };

        conn.write_line(&response).await?;
    }
    Ok(())
}

fn key_arg(args: &str) -> Option<&str> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(key), None) => Some(key),
        _ => None,
    }
}

fn set_args(args: &str) -> Option<(&str, &str)> {
    let args = args.trim_start();
    let separator = args.find(char::is_whitespace)?;
    let key = &args[..separator];
    let value = args[separator..].trim_start();
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

fn arity_error(command: &str) -> String {
    format!("-ERR wrong number of arguments for '{command}'")
}
