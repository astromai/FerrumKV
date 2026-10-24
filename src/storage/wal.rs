//! Write-Ahead Log: una operación por línea, terminada en `\n`.
//!
//! ```text
//! SET <clave> <valor>
//! DEL <clave>
//! INCR <clave>
//! ```
//!
//! La clave no tiene whitespace; el valor es el resto de la línea, sin `\n` ni `\r`.
//! Una última línea sin `\n` es [`RecordError::Truncated`].

use std::error::Error;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Set { key: String, value: String },
    Del { key: String },
    Incr { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    Truncated,
    InvalidUtf8,
    EmptyLine,
    UnknownOp(String),
    MissingArgument,
    InvalidKey,
    InvalidValue,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::Truncated => write!(f, "última línea incompleta (falta '\\n')"),
            RecordError::InvalidUtf8 => write!(f, "la línea no es UTF-8 válido"),
            RecordError::EmptyLine => write!(f, "línea vacía"),
            RecordError::UnknownOp(tag) => write!(f, "operación desconocida '{tag}'"),
            RecordError::MissingArgument => write!(f, "faltan argumentos"),
            RecordError::InvalidKey => write!(f, "clave vacía o con whitespace"),
            RecordError::InvalidValue => write!(f, "valor con '\\n' o '\\r'"),
        }
    }
}

impl Error for RecordError {}

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    /// La operación no se puede representar; no se escribió nada.
    InvalidOp(RecordError),
    /// Registro inválido en `line` (desde 1).
    Corrupt {
        line: usize,
        kind: RecordError,
    },
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalError::Io(err) => write!(f, "error de E/S en el WAL: {err}"),
            WalError::InvalidOp(kind) => {
                write!(f, "operación no representable en el WAL: {kind}")
            }
            WalError::Corrupt { line, kind } => {
                write!(f, "WAL corrupto en la línea {line}: {kind}")
            }
        }
    }
}

impl Error for WalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            WalError::Io(err) => Some(err),
            WalError::InvalidOp(kind) | WalError::Corrupt { kind, .. } => Some(kind),
        }
    }
}

impl From<io::Error> for WalError {
    fn from(err: io::Error) -> Self {
        WalError::Io(err)
    }
}

#[derive(Debug)]
pub struct Wal {
    file: File,
    path: PathBuf,
}

impl Wal {
    /// Abre el WAL en `path`, creándolo si no existe.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Anexa `op` y hace `sync_data` antes de devolver `Ok`.
    pub fn append(&mut self, op: &Op) -> Result<(), WalError> {
        let line = encode(op).map_err(WalError::InvalidOp)?;
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }

    /// Devuelve las operaciones en orden; falla en el primer registro inválido.
    pub fn read_all(&self) -> Result<Vec<Op>, WalError> {
        let mut reader = BufReader::new(File::open(&self.path)?);
        let mut ops = Vec::new();
        let mut buf = Vec::new();
        let mut line = 0;
        loop {
            buf.clear();
            if reader.read_until(b'\n', &mut buf)? == 0 {
                return Ok(ops);
            }
            line += 1;
            let op = parse_record(&buf).map_err(|kind| WalError::Corrupt { line, kind })?;
            ops.push(op);
        }
    }
}

fn encode(op: &Op) -> Result<String, RecordError> {
    match op {
        Op::Set { key, value } => {
            validate_key(key)?;
            validate_value(value)?;
            Ok(format!("SET {key} {value}\n"))
        }
        Op::Del { key } => {
            validate_key(key)?;
            Ok(format!("DEL {key}\n"))
        }
        Op::Incr { key } => {
            validate_key(key)?;
            Ok(format!("INCR {key}\n"))
        }
    }
}

fn parse_record(bytes: &[u8]) -> Result<Op, RecordError> {
    let bytes = bytes.strip_suffix(b"\n").ok_or(RecordError::Truncated)?;
    let line = std::str::from_utf8(bytes).map_err(|_| RecordError::InvalidUtf8)?;
    decode(line)
}

fn decode(line: &str) -> Result<Op, RecordError> {
    if line.is_empty() {
        return Err(RecordError::EmptyLine);
    }
    let (tag, args) = match line.split_once(' ') {
        Some((tag, args)) => (tag, Some(args)),
        None => (line, None),
    };
    match tag {
        "SET" => {
            let (key, value) = args
                .and_then(|args| args.split_once(' '))
                .ok_or(RecordError::MissingArgument)?;
            validate_key(key)?;
            validate_value(value)?;
            Ok(Op::Set {
                key: key.to_string(),
                value: value.to_string(),
            })
        }
        "DEL" => Ok(Op::Del {
            key: parse_key(args)?,
        }),
        "INCR" => Ok(Op::Incr {
            key: parse_key(args)?,
        }),
        other => Err(RecordError::UnknownOp(other.to_string())),
    }
}

fn parse_key(args: Option<&str>) -> Result<String, RecordError> {
    let key = args.ok_or(RecordError::MissingArgument)?;
    validate_key(key)?;
    Ok(key.to_string())
}

fn validate_key(key: &str) -> Result<(), RecordError> {
    if key.is_empty() || key.contains(char::is_whitespace) {
        return Err(RecordError::InvalidKey);
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<(), RecordError> {
    if value.contains(['\n', '\r']) {
        return Err(RecordError::InvalidValue);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Ruta temporal única; el archivo se borra en `drop`.
    struct TempPath {
        path: PathBuf,
    }

    impl TempPath {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let file = format!("ferrum_wal_{name}_{}_{n}.log", std::process::id());
            let path = std::env::temp_dir().join(file);
            let _ = std::fs::remove_file(&path);
            Self { path }
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn set(key: &str, value: &str) -> Op {
        Op::Set {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn del(key: &str) -> Op {
        Op::Del {
            key: key.to_string(),
        }
    }

    fn incr(key: &str) -> Op {
        Op::Incr {
            key: key.to_string(),
        }
    }

    fn corrupt_kind(contents: &[u8]) -> (usize, RecordError) {
        let tmp = TempPath::new("corrupto");
        std::fs::write(&tmp.path, contents).unwrap();
        let wal = Wal::open(&tmp.path).unwrap();
        match wal.read_all() {
            Err(WalError::Corrupt { line, kind }) => (line, kind),
            other => panic!(
                "se esperaba WalError::Corrupt para {:?}, se obtuvo {other:?}",
                String::from_utf8_lossy(contents)
            ),
        }
    }

    #[test]
    fn open_crea_archivo_nuevo_vacio() {
        let tmp = TempPath::new("nuevo");
        assert!(!tmp.path.exists());

        let wal = Wal::open(&tmp.path).unwrap();

        assert!(tmp.path.exists());
        assert_eq!(wal.path(), tmp.path.as_path());
        assert_eq!(wal.read_all().unwrap(), Vec::new());
    }

    #[test]
    fn append_varias_operaciones_y_leer_en_orden() {
        let tmp = TempPath::new("orden");
        let mut wal = Wal::open(&tmp.path).unwrap();
        let ops = [set("a", "1"), incr("a"), set("b", "x"), del("a"), incr("c")];

        for op in &ops {
            wal.append(op).unwrap();
        }

        assert_eq!(wal.read_all().unwrap(), ops);
    }

    #[test]
    fn reabrir_recupera_en_orden_y_sigue_anexando() {
        let tmp = TempPath::new("reabrir");
        {
            let mut wal = Wal::open(&tmp.path).unwrap();
            wal.append(&set("a", "1")).unwrap();
            wal.append(&incr("a")).unwrap();
        }

        let mut wal = Wal::open(&tmp.path).unwrap();
        assert_eq!(wal.read_all().unwrap(), [set("a", "1"), incr("a")]);
        wal.append(&del("a")).unwrap();
        drop(wal);

        let wal = Wal::open(&tmp.path).unwrap();
        assert_eq!(
            wal.read_all().unwrap(),
            [set("a", "1"), incr("a"), del("a")]
        );
    }

    #[test]
    fn valores_con_espacios_se_conservan_exactos() {
        let tmp = TempPath::new("espacios");
        let mut wal = Wal::open(&tmp.path).unwrap();
        let values = [
            "hola mundo",
            "  dos  espacios  ",
            "con\ttab",
            "ñandú 🦀",
            " ",
            "",
        ];
        let ops: Vec<Op> = values
            .iter()
            .enumerate()
            .map(|(i, value)| set(&format!("k{i}"), value))
            .collect();

        for op in &ops {
            wal.append(op).unwrap();
        }

        assert_eq!(wal.read_all().unwrap(), ops);
    }

    #[test]
    fn formato_en_disco_es_exacto() {
        let tmp = TempPath::new("formato");
        let mut wal = Wal::open(&tmp.path).unwrap();
        wal.append(&set("saludo", "hola mundo")).unwrap();
        wal.append(&incr("visitas")).unwrap();
        wal.append(&del("saludo")).unwrap();
        wal.append(&set("vacio", "")).unwrap();

        let contents = std::fs::read_to_string(&tmp.path).unwrap();
        assert_eq!(
            contents,
            "SET saludo hola mundo\nINCR visitas\nDEL saludo\nSET vacio \n"
        );
    }

    #[test]
    fn encode_y_decode_son_inversos() {
        for op in [set("k", "hola  mundo "), set("k", ""), del("k"), incr("k")] {
            let line = encode(&op).unwrap();
            assert_eq!(parse_record(line.as_bytes()), Ok(op));
        }
    }

    #[test]
    fn corrupto_operacion_desconocida() {
        assert_eq!(
            corrupt_kind(b"SET a 1\nPUT a 2\n"),
            (2, RecordError::UnknownOp("PUT".to_string()))
        );
        assert_eq!(
            corrupt_kind(b"set a 1\n"),
            (1, RecordError::UnknownOp("set".to_string()))
        );
    }

    #[test]
    fn corrupto_ultima_linea_truncada() {
        assert_eq!(
            corrupt_kind(b"SET a 1\nSET b 2"),
            (2, RecordError::Truncated)
        );
    }

    #[test]
    fn corrupto_argumentos_faltantes_o_sobrantes() {
        let cases: [(&[u8], RecordError); 7] = [
            (b"SET\n", RecordError::MissingArgument),
            (b"SET k\n", RecordError::MissingArgument),
            (b"DEL\n", RecordError::MissingArgument),
            (b"INCR\n", RecordError::MissingArgument),
            (b"DEL a b\n", RecordError::InvalidKey),
            (b"INCR \n", RecordError::InvalidKey),
            (b"SET  v\n", RecordError::InvalidKey),
        ];
        for (contents, expected) in cases {
            assert_eq!(corrupt_kind(contents), (1, expected));
        }
    }

    #[test]
    fn corrupto_linea_vacia() {
        assert_eq!(
            corrupt_kind(b"SET a 1\n\nDEL a\n"),
            (2, RecordError::EmptyLine)
        );
    }

    #[test]
    fn corrupto_fin_de_linea_crlf() {
        assert_eq!(corrupt_kind(b"SET a 1\r\n"), (1, RecordError::InvalidValue));
        assert_eq!(corrupt_kind(b"DEL a\r\n"), (1, RecordError::InvalidKey));
    }

    #[test]
    fn corrupto_utf8_invalido() {
        assert_eq!(
            corrupt_kind(b"SET a 1\nSET b \xff\xfe\n"),
            (2, RecordError::InvalidUtf8)
        );
    }

    #[test]
    fn append_rechaza_operaciones_no_representables_sin_tocar_el_archivo() {
        let tmp = TempPath::new("rechazo");
        let mut wal = Wal::open(&tmp.path).unwrap();
        wal.append(&set("a", "1")).unwrap();

        let invalid = [
            (set("", "v"), RecordError::InvalidKey),
            (set("con espacio", "v"), RecordError::InvalidKey),
            (del("a\tb"), RecordError::InvalidKey),
            (incr("a\nb"), RecordError::InvalidKey),
            (set("k", "linea1\nlinea2"), RecordError::InvalidValue),
            (set("k", "v\r"), RecordError::InvalidValue),
        ];
        for (op, expected) in invalid {
            match wal.append(&op) {
                Err(WalError::InvalidOp(kind)) => assert_eq!(kind, expected, "op: {op:?}"),
                other => panic!("se esperaba WalError::InvalidOp para {op:?}, se obtuvo {other:?}"),
            }
        }

        assert_eq!(std::fs::read_to_string(&tmp.path).unwrap(), "SET a 1\n");
    }

    #[test]
    fn mensaje_de_error_incluye_la_linea() {
        let err = WalError::Corrupt {
            line: 3,
            kind: RecordError::Truncated,
        };
        assert_eq!(
            err.to_string(),
            "WAL corrupto en la línea 3: última línea incompleta (falta '\\n')"
        );
    }
}
