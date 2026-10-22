use std::fmt;

/// Representación de comandos soportados
pub enum Command {
    Ping(),
    DBSIZE,
    QUIT,
    SET { key: String, val: String },
    GET(String),
    DEL(String),
    INCR(String),
}
