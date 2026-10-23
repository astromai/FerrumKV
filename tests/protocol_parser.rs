use ferrum_kv::protocol::{Command, ParseError};

#[test]
fn test_empty_command() {
    assert_eq!(Command::parse(""), Err(ParseError::EmptyCommand));
    assert_eq!(Command::parse("   \t  "), Err(ParseError::EmptyCommand));
}

#[test]
fn test_unknown_command() {
    assert_eq!(
        Command::parse("AYOO key"),
        Err(ParseError::UnknownCommand("AYOO".to_string()))
    );
}

#[test]
fn test_ping_without_and_with_args() {
    assert_eq!(Command::parse("PING"), Ok(Command::Ping(None)));
    assert_eq!(Command::parse("ping"), Ok(Command::Ping(None)));
    assert_eq!(
        Command::parse("PING hello world"),
        Ok(Command::Ping(Some("hello world".to_string())))
    );
    assert_eq!(
        Command::parse("ping   pong  "),
        Ok(Command::Ping(Some("pong".to_string())))
    );
}

#[test]
fn test_dbsize_and_quit() {
    assert_eq!(Command::parse("DBSIZE"), Ok(Command::DBSIZE));
    assert_eq!(Command::parse("dbsize"), Ok(Command::DBSIZE));
    assert_eq!(Command::parse("QUIT"), Ok(Command::QUIT));
    assert_eq!(Command::parse("quit"), Ok(Command::QUIT));

    // Errores por argumentos extra
    assert!(matches!(
        Command::parse("DBSIZE alo"),
        Err(ParseError::WrongNumberOfArguments { expected: 0, got: 1, .. })
    ));
    assert!(matches!(
        Command::parse("QUIT now"),
        Err(ParseError::WrongNumberOfArguments { expected: 0, got: 1, .. })
    ));
}

#[test]
fn test_set_valid_and_preserves_spaces() {
    assert_eq!(
        Command::parse("SET mykey 123"),
        Ok(Command::SET {
            key: "mykey".to_string(),
            val: "123".to_string(),
        })
    );
    assert_eq!(
        Command::parse("set saludo hola chat con espacios"),
        Ok(Command::SET {
            key: "saludo".to_string(),
            val: "hola chat con espacios".to_string(),
        })
    );
}

#[test]
fn test_set_wrong_arguments() {
    assert!(matches!(
        Command::parse("SET"),
        Err(ParseError::WrongNumberOfArguments { expected: 2, got: 0, .. })
    ));
    assert!(matches!(
        Command::parse("SET onlykey"),
        Err(ParseError::WrongNumberOfArguments { expected: 2, got: 1, .. })
    ));
    assert!(matches!(
        Command::parse("SET onlykey   "),
        Err(ParseError::WrongNumberOfArguments { expected: 2, got: 1, .. })
    ));
}

#[test]
fn test_single_arg_commands() {
    assert_eq!(Command::parse("GET x"), Ok(Command::GET("x".to_string())));
    assert_eq!(Command::parse("get x"), Ok(Command::GET("x".to_string())));
    assert_eq!(Command::parse("DEL x"), Ok(Command::DEL("x".to_string())));
    assert_eq!(Command::parse("del x"), Ok(Command::DEL("x".to_string())));
    assert_eq!(Command::parse("INCR c"), Ok(Command::INCR("c".to_string())));
    assert_eq!(Command::parse("incr c"), Ok(Command::INCR("c".to_string())));
}

#[test]
fn test_single_arg_errors() {
    assert!(matches!(
        Command::parse("GET"),
        Err(ParseError::WrongNumberOfArguments { expected: 1, got: 0, .. })
    ));
    assert!(matches!(
        Command::parse("GET a b"),
        Err(ParseError::WrongNumberOfArguments { expected: 1, got: 2, .. })
    ));
    assert!(matches!(
        Command::parse("DEL a b c"),
        Err(ParseError::WrongNumberOfArguments { expected: 1, got: 3, .. })
    ));
}
