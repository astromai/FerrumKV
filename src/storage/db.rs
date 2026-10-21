use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Base de datos compartida entre todas las conexiones.
///
/// `Arc`  -> permite que muchas tareas tengan un "puntero" al mismo estado.
/// `Mutex` -> garantiza que solo una tarea a la vez modifique el HashMap.
///
/// Se usa `std::sync::Mutex` (no el de Tokio) porque el candado se toma por
/// un instante y NUNCA se mantiene a través de un `.await`. Es lo que recomienda
/// la propia documentación de Tokio para este caso: es más rápido.
#[derive(Clone, Default)]
pub struct Db {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl Db {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cantidad de claves almacenadas (útil para tests y depuración).
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // En el siguiente paso aquí agregaremos: set, get, del, incr.
}
