use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

/// Envuelve un `TcpStream` y ofrece lectura por líneas con buffer.
///
/// Por ahora el protocolo es texto plano terminado en `\n` (como los
/// "inline commands" de Redis, que se pueden probar con `nc` o `telnet`).
/// Más adelante se puede reemplazar por RESP sin tocar el servidor.
pub struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    line: String,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        let (r, w) = stream.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
            line: String::new(),
        }
    }

    /// Lee la siguiente línea. Devuelve `Ok(None)` si el cliente cerró la conexión.
    pub async fn read_line(&mut self) -> std::io::Result<Option<&str>> {
        self.line.clear();
        let n = self.reader.read_line(&mut self.line).await?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(self.line.trim_end_matches(['\r', '\n'])))
    }

    /// Escribe una respuesta terminada en `\r\n` (igual que Redis).
    pub async fn write_line(&mut self, msg: &str) -> std::io::Result<()> {
        self.writer.write_all(msg.as_bytes()).await?;
        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await
    }
}
