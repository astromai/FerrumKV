# FerrumKV

Servidor Key-Value inspirado en Redis, desarrollado en Rust como proyecto del curso de Programación Avanzada.

El servidor atiende múltiples clientes simultáneos por TCP y mantiene los datos en memoria, compartidos de forma segura entre conexiones.

## Integrantes

- Tomás Ubilla
- Rodrigo Díaz
- Felipe Cárdenas
- Daniela Espinoza

## Estado del proyecto


## Requisitos

- Rust 1.70 o superior (se recomienda la versión estable más reciente)
- Cargo

## Uso

Compilar y ejecutar el servidor (por defecto en `127.0.0.1:6379`, el mismo puerto que Redis):

```bash
cargo run
```

Usar otro puerto:

```bash
cargo run -- 7000
```

Conectarse como cliente desde otra terminal (se pueden abrir varias a la vez):

```bash
nc 127.0.0.1 6379
```

Ejemplo de sesión:

```
PING
+PONG
PING hola
hola
DBSIZE
:0
QUIT
+OK
```

Para detener el servidor, presionar `Ctrl+C`.

## Comandos disponibles

El protocolo actual es texto plano: un comando por línea. Las respuestas imitan el formato de Redis (`+` para respuestas simples, `:` para enteros y `-ERR` para errores).

| Comando        | Descripción                                   | Respuesta              |
|----------------|-----------------------------------------------|------------------------|
| `PING`         | Verifica la conexión                          | `+PONG`                |
| `PING mensaje` | Devuelve el mensaje recibido                  | `mensaje`              |
| `DBSIZE`       | Cantidad de claves almacenadas                | `:N`                   |
| `QUIT`         | Cierra la conexión                            | `+OK`                  |

## Estructura del proyecto

```
rusty_kv/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs           # Punto de entrada: lee argumentos y arranca el servidor
│   ├── lib.rs            # Declaración de módulos y constantes globales
│   ├── server.rs         # Acepta conexiones y lanza una tarea por cliente
│   ├── network/
│   │   ├── mod.rs
│   │   └── connection.rs # Lectura y escritura de líneas sobre el socket
│   └── storage/
│       ├── mod.rs
│       └── db.rs         # Base de datos en memoria compartida entre tareas
└── tests/
    └── concurrency.rs    # Pruebas de integración con clientes reales
```

La lógica vive en la librería (`lib.rs` y sus módulos), y `main.rs` solo arranca el servidor. Esto permite que las pruebas de `tests/` levanten el servidor directamente.

Las dependencias entre módulos van en una sola dirección: `server` usa `network` y `storage`, y `storage` no depende de ningún otro módulo.

## Decisiones de diseño

### Concurrencia: Tokio en lugar de threads

Se evaluaron dos alternativas: programación asíncrona con Tokio y un thread del sistema operativo por cliente. Se eligió **Tokio** por las siguientes razones:

- **El trabajo es mayormente espera de red.** Las operaciones sobre el `HashMap` toman nanosegundos; el resto del tiempo el servidor espera datos de los clientes. En ese escenario, una tarea asíncrona suspendida no consume CPU.
- **Escalabilidad.** Cada thread reserva un stack de varios MB y los cambios de contexto pasan por el kernel. Una tarea de Tokio pesa unos pocos KB, por lo que miles de conexiones caben en un puñado de threads.
- **Similitud con Redis.** Redis usa un event loop (epoll) de un solo hilo, que es el mismo modelo que Tokio implementa internamente.

Los threads siguen siendo una opción válida para pocos clientes y son más simples de leer, pero no escalan igual.

### Estado compartido: `Arc<Mutex<HashMap>>`

- `Arc` permite que todas las tareas compartan el mismo mapa. Clonar `Db` solo incrementa un contador de referencias.
- `Mutex` garantiza que una sola tarea modifique el mapa a la vez.
- Se usa `std::sync::Mutex` y no `tokio::sync::Mutex`, porque el candado se mantiene por un instante y **nunca a través de un `.await`**. En ese caso el Mutex estándar es más rápido, tal como recomienda la documentación de Tokio.

### Límite de conexiones

Un `Semaphore` limita los clientes simultáneos a `MAX_CONNECTIONS` (1000), de forma similar a la opción `maxclients` de Redis. Así se evita agotar memoria o descriptores de archivo.

### Apagado ordenado

El servidor recibe un future de apagado (`Ctrl+C` en producción).

## Pruebas

```bash
cargo test
```

Pruebas incluidas en `tests/concurrency.rs`:

- `ping_responde_pong`: verifica las respuestas de `PING` y el error ante comandos desconocidos.
- `muchos_clientes_simultaneos`: conecta 200 clientes en paralelo, cada uno envía 50 comandos y comprueba que cada cliente reciba exactamente sus propias respuestas.

## Próximos pasos

