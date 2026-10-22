use std::fmt;

/// Representación de comandos soportados
pub enum Command {
    Ping(Option<String>),
    DBSIZE,
    QUIT,
    SET { key: String, val: String },
    GET(String),
    DEL(String),
    INCR(String),
}

pub enum ParseError {
    EmptyCommand,
    UnknownCommand(String),
    WrongNumberOfArguments {
        command: String,
        expected: usize,
        got: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::EmptyCommand => write!(f, "[ERROR] empty command"),
            ParseError::UnknownCommand => write!(f, "[ERROR] unknown command '{id}'"),
            ParseError::WrongNumberOfArguments {command, expected, got} => write!(f, "[ERROR] wrong number of arguments for '{command}'; command (expected {expected}, got {got})'"),
        }
    }
}

impl std::error::Error for ParseError {}
