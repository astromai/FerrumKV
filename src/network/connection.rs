use std::io::{Error, ErrorKind};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

/// Envuelve un `TcpStream` y ofrece lectura por líneas con buffer.
///
/// Acepta un comando por línea, terminado en `\n` o `\r\n`
/// (como los "inline commands" de Redis).
///
/// La lectura está acotada por [`Connection::MAX_LINE_LENGTH`]. Un cliente no
/// puede hacer crecer la memoria del servidor enviando una línea enorme o un
/// flujo de bytes sin salto de línea.
pub struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    buf: Vec<u8>,
    line: String,
}

impl Connection {
    pub const MAX_LINE_LENGTH: usize = 16 * 1024;

    pub fn new(stream: TcpStream) -> Self {
        let (r, w) = stream.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
            buf: Vec::new(),
            line: String::new(),
        }
    }

    pub async fn read_line(&mut self) -> std::io::Result<Option<&str>> {
        self.buf.clear();

        // Se permite 1 byte extra para el `\r` de un terminador `\r\n`.
        let raw_limit = Self::MAX_LINE_LENGTH + 1;

        loop {
            let available = self.reader.fill_buf().await?;

            // El cliente cerró la conexión.
            if available.is_empty() {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                break;
            }

            match available.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    if self.buf.len() + pos > raw_limit {
                        return Err(line_too_long());
                    }
                    self.buf.extend_from_slice(&available[..pos]);
                    self.reader.consume(pos + 1); // incluye el '\n'
                    break;
                }
                None => {
                    let len = available.len();
                    if self.buf.len() + len > raw_limit {
                        // Ya no puede ser una línea válida, no sigue acumulando bytes.
                        return Err(line_too_long());
                    }
                    self.buf.extend_from_slice(available);
                    self.reader.consume(len);
                }
            }
        }

        // Quita los `\r` finales.
        while self.buf.last() == Some(&b'\r') {
            self.buf.pop();
        }

        // El `\r` extra permitido arriba no puede usarse como contenido.
        if self.buf.len() > Self::MAX_LINE_LENGTH {
            return Err(line_too_long());
        }

        self.line.clear();
        let text =
            std::str::from_utf8(&self.buf).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        self.line.push_str(text);

        Ok(Some(&self.line))
    }

    pub async fn write_line(&mut self, msg: &str) -> std::io::Result<()> {
        self.writer.write_all(msg.as_bytes()).await?;
        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await
    }
}

// Devuelve error para el caso en que el comando inline supera el tamaño máximo
// de bytes [`Connection::MAX_LINE_LENGTH`]
fn line_too_long() -> Error {
    Error::new(
        ErrorKind::InvalidData,
        format!(
            "comando demasiado largo (máximo {} bytes)",
            Connection::MAX_LINE_LENGTH
        ),
    )
}
