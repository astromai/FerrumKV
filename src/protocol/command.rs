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

impl Command {
    /// parsea una línea completa devolviendo el comando tipado o un error explícito
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ParseError::EmptyCommand);
        }
        let (id, args) = 
            match trimmed.split_once(char::is_whitespace){
                Some((id, args)) => (id, Some(args.trim_start())),
                None => (trimmed, None),
        };
        let upper_id = id.to_ascii_uppercase();
        match upper_id.as_str() {
            "PING" => {
                match args {
                    None => Ok(Command::Ping(None)),
                    Some(msg) => {
                        if msg.is_empty() {
                            Ok(Command::Ping(None))
                        } else {
                            Ok(Command::Ping(msg.to_string()))
                        }
                    }
                }
            }
            "DBSIZE" => {
                match args {
                Some(extra) if extra.split_whitespace().count() > 0{
                    Err(ParseError::WrongNumberOfArguments {
                        command: upper_id,
                        expected: 0,
                        got: extra.split_whitespace().count(),
                    })
                }
                _ => Ok(Command::DBSIZE),
                }
            }
            "QUIT" => {
                match args {
                Some(extra) if extra.split_whitespace().count() > 0{
                    Err(ParseError::WrongNumberOfArguments {
                        command: upper_id,
                        expected: 0,
                        got: extra.split_whitespace().count(),
                    })
                }
                _ => Ok(Command::QUIT),
                }
            }
            "SET" => {
                todo!();
            }
            "GET" => {
                todo!();
            }
            "DEL" => {
                todo!();
            }
            "INCR" => {
                todo!();
            }
            _ => Err(ParseError::UnknownCommand(upper_id.to_string())),
        }
    }
}
