//! Write-Ahead Log (WAL): registro durable de las operaciones que modifican el
//! estado, para poder reconstruirlo después de reiniciar el servidor.
//!
//! Este módulo es autocontenido: no conoce `Db` ni el servidor. Solo sabe
//! anexar operaciones a un archivo y leerlas de vuelta en el mismo orden.
//!
//! # Formato del archivo
//!
//! Texto UTF-8, un registro por línea, cada uno terminado en `\n` (LF):
//!
//! ```text
//! SET <clave> <valor>
//! DEL <clave>
//! INCR <clave>
//! ```
//!
//! - La etiqueta va en mayúsculas exactas y la separa de la clave un único
//!   espacio.
//! - La clave no puede ser vacía ni contener whitespace. Es la misma regla del
//!   protocolo inline, donde la clave es el primer token de la línea.
//! - El valor es *todo* lo que sigue a `SET <clave> ` hasta el `\n`, sin
//!   recortar: `SET saludo hola mundo` guarda `"hola mundo"`. Puede ser vacío
//!   (`SET k ` seguido de `\n`) y puede contener espacios y tabs, pero no `\n`
//!   ni `\r`.
//! - Un archivo vacío representa cero operaciones.
//!
//! Ejemplo de un WAL válido:
//!
//! ```text
//! SET saludo hola mundo
//! INCR visitas
//! DEL saludo
//! ```
//!
//! La serialización es determinista: la misma operación produce siempre los
//! mismos bytes. Las claves o valores que el formato no puede representar se
//! rechazan en [`Wal::append`] con [`WalError::InvalidOp`], sin tocar el
//! archivo. Se prefirió rechazar antes que escapar caracteres porque el
//! protocolo inline nunca produce esos casos y así el archivo se puede leer
//! con `cat`.
//!
//! # Registros corruptos
//!
//! [`Wal::read_all`] nunca ignora un registro inválido: devuelve
//! [`WalError::Corrupt`] con el número de línea (desde 1) y el motivo.
//!
//! Una última línea sin `\n` se reporta como [`RecordError::Truncated`]. Lo
//! más probable es que el proceso se haya caído en medio de un `append`, pero
//! desde el archivo no se puede distinguir eso de un daño real. Además,
//! `SET k hola mun` parece un registro válido: solo la falta del `\n` revela
//! que está incompleto. Por eso se reporta como error en vez de adivinar.
//!
//! Qué hacer con ese error lo decide la integración con el servidor (no
//! implementada aquí). Una política razonable es descartar solo esa última
//! línea y truncar el archivo hasta el último `\n`: como ese `append` nunca
//! devolvió `Ok`, la operación nunca se confirmó al cliente.
//!
//! # Durabilidad
//!
//! Cada [`Wal::append`] escribe el registro completo con una sola llamada a
//! `write_all` y luego llama a [`File::sync_data`] antes de devolver `Ok`.
//!
//! - `flush()` no basta: en un `File` no hace nada (no tiene buffer propio) y,
//!   con un `BufWriter`, solo entrega los datos al sistema operativo, donde
//!   pueden perderse ante un corte de luz.
//! - `sync_data()` espera a que los datos y el tamaño del archivo lleguen al
//!   disco. `sync_all()` además sincroniza metadatos como la fecha de
//!   modificación, que aquí no se necesitan y solo agregan costo.
//! - Costo: cada operación mutable espera al disco, lo que limita el
//!   throughput de escrituras.
//!
//! Limitaciones conocidas:
//!
//! - Para que la *creación* del archivo sobreviva a un corte de luz también
//!   habría que sincronizar el directorio padre (solo posible en Unix). No se
//!   hace.
//! - Si `append` falla con [`WalError::Io`], puede haber quedado un registro a
//!   medias al final del archivo. El llamador no debe seguir escribiendo en
//!   ese `Wal`: el siguiente registro quedaría pegado al incompleto.
//! - `Wal` no se sincroniza internamente. Para compartirlo entre tareas hay
//!   que envolverlo, por ejemplo en un `Mutex`.

use std::error::Error;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Operación que modifica el estado y que el WAL sabe persistir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Almacena o reemplaza `value` en `key`.
    Set { key: String, value: String },
    /// Elimina `key`.
    Del { key: String },
    /// Incrementa en uno el valor numérico de `key`.
    ///
    /// Se guarda la operación, no su resultado: al recuperar, el `INCR` se
    /// vuelve a aplicar sobre el estado reconstruido hasta ese punto.
    Incr { key: String },
}

/// Motivo por el que un registro no es válido.
///
/// Se usa al escribir ([`WalError::InvalidOp`]) y al leer
/// ([`WalError::Corrupt`]), porque ambas direcciones aplican las mismas reglas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// La última línea del archivo no termina en `\n`.
    Truncated,
    /// La línea no es UTF-8 válido.
    InvalidUtf8,
    /// Línea vacía.
    EmptyLine,
    /// Etiqueta distinta de `SET`, `DEL` o `INCR`.
    UnknownOp(String),
    /// Falta la clave o, en `SET`, el valor.
    MissingArgument,
    /// Clave vacía o con whitespace.
    InvalidKey,
    /// Valor con `\n` o `\r`.
    InvalidValue,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::Truncated => write!(f, "incomplete last line (missing '\\n')"),
            RecordError::InvalidUtf8 => write!(f, "line is not valid UTF-8"),
            RecordError::EmptyLine => write!(f, "empty line"),
            RecordError::UnknownOp(tag) => write!(f, "unknown operation '{tag}'"),
            RecordError::MissingArgument => write!(f, "missing arguments"),
            RecordError::InvalidKey => write!(f, "empty key or key contains whitespace"),
            RecordError::InvalidValue => write!(f, "value contains '\\n' or '\\r'"),
        }
    }
}

impl Error for RecordError {}

/// Error del WAL.
///
/// Distingue un fallo del sistema operativo ([`WalError::Io`]) de un problema
/// con el contenido ([`WalError::InvalidOp`] al escribir y
/// [`WalError::Corrupt`] al leer).
#[derive(Debug)]
pub enum WalError {
    /// Error de entrada/salida del sistema operativo.
    Io(io::Error),
    /// La operación no se puede representar en el formato; no se escribió nada.
    InvalidOp(RecordError),
    /// El archivo contiene un registro inválido en `line` (contando desde 1).
    Corrupt { line: usize, kind: RecordError },
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalError::Io(err) => write!(f, "WAL I/O error: {err}"),
            WalError::InvalidOp(kind) => {
                write!(f, "operation cannot be represented in the WAL: {kind}")
            }
            WalError::Corrupt { line, kind } => {
                write!(f, "corrupt WAL at line {line}: {kind}")
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

/// Permite usar `?` sobre operaciones de `std::io` dentro del módulo.
impl From<io::Error> for WalError {
    fn from(err: io::Error) -> Self {
        WalError::Io(err)
    }
}

/// Write-Ahead Log respaldado por un archivo.
///
/// `append` toma `&mut self`, así que el compilador impide escribir desde dos
/// lugares a la vez sin sincronización explícita (por ejemplo, un `Mutex`).
#[derive(Debug)]
pub struct Wal {
    file: File,
    path: PathBuf,
}

impl Wal {
    /// Abre el WAL en `path`, creándolo vacío si no existe.
    ///
    /// Las escrituras siempre se anexan al final: el contenido existente no se
    /// modifica.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { file, path })
    }

    /// Ruta del archivo del WAL.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Anexa `op` al final del archivo y espera a que llegue al disco.
    ///
    /// Solo devuelve `Ok` después de `sync_data`, así que una operación
    /// confirmada sobrevive a un corte de luz. Si devuelve
    /// [`WalError::InvalidOp`], el archivo no se modificó. Si devuelve
    /// [`WalError::Io`], puede haber quedado un registro a medias y no se debe
    /// seguir escribiendo (ver la documentación del módulo).
    pub fn append(&mut self, op: &Op) -> Result<(), WalError> {
        let line = encode(op).map_err(WalError::InvalidOp)?;
        // Una sola escritura con el registro completo, incluido su '\n'.
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }

    /// Lee el archivo completo y devuelve las operaciones en el orden en que
    /// se registraron.
    ///
    /// Se detiene en el primer registro inválido con [`WalError::Corrupt`];
    /// nunca omite registros.
    pub fn read_all(&self) -> Result<Vec<Op>, WalError> {
        let mut reader = BufReader::new(File::open(&self.path)?);
        let mut ops = Vec::new();
        // Se lee en bytes (no con `lines()`) para conservar el '\n' final, que
        // es lo que permite detectar una última línea truncada, y para
        // reportar UTF-8 inválido como corrupción con número de línea.
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

/// Serializa `op` como una línea terminada en `\n`.
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

/// Convierte los bytes de una línea leída (incluido su `\n`) en una operación.
fn parse_record(bytes: &[u8]) -> Result<Op, RecordError> {
    let bytes = bytes.strip_suffix(b"\n").ok_or(RecordError::Truncated)?;
    let line = std::str::from_utf8(bytes).map_err(|_| RecordError::InvalidUtf8)?;
    decode(line)
}

/// Interpreta una línea sin su `\n` final.
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
            // Solo se corta en el primer espacio después de la clave: el resto,
            // con todos sus espacios, es el valor.
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

/// Valida el único argumento de `DEL` e `INCR`. Un argumento extra (`DEL a b`)
/// deja un espacio dentro de la clave y se rechaza como clave inválida.
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

    /// Ruta única dentro del directorio temporal del sistema. El archivo se
    /// borra al salir del test, incluso si el test falla (panic).
    struct TempPath {
        path: PathBuf,
    }

    impl TempPath {
        fn new(name: &str) -> Self {
            // El pid separa ejecuciones simultáneas de `cargo test`; el
            // contador separa los tests de una misma ejecución, que corren en
            // paralelo.
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let file = format!("ferrum_wal_{name}_{}_{n}.log", std::process::id());
            let path = std::env::temp_dir().join(file);
            // Por si quedó un archivo de una ejecución anterior con el mismo pid.
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

    /// Escribe un WAL a mano y devuelve la línea y el motivo de la corrupción.
    fn corrupt_kind(contents: &[u8]) -> (usize, RecordError) {
        let tmp = TempPath::new("corrupto");
        std::fs::write(&tmp.path, contents).unwrap();
        let wal = Wal::open(&tmp.path).unwrap();
        match wal.read_all() {
            Err(WalError::Corrupt { line, kind }) => (line, kind),
            other => panic!(
                "expected WalError::Corrupt for {:?}, got {other:?}",
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
        } // El `Wal` se destruye aquí y el archivo se cierra.

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
        // La etiqueta distingue mayúsculas: el WAL lo escribe solo el servidor.
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
        // Un WAL editado en Windows deja '\r' antes de cada '\n'.
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
                other => panic!("expected WalError::InvalidOp for {op:?}, got {other:?}"),
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
            "corrupt WAL at line 3: incomplete last line (missing '\\n')"
        );
    }
}
