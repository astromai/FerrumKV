//! Servidor Key-Value inspirado en Redis.
//!
//! Módulos:
//! - `server`: acepta conexiones TCP y lanza una tarea por cliente.
//! - `connection`: lee/escribe líneas sobre un socket.
//! - `db`: estado compartido en memoria, seguro entre tareas.

pub mod network;
pub mod server;
pub mod storage;

/// Puerto por defecto (el mismo que usa Redis).
pub const DEFAULT_PORT: u16 = 6379;

/// Máximo de clientes simultáneos (Redis usa `maxclients`, por defecto 10000).
pub const MAX_CONNECTIONS: usize = 1000;
