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

    /// Guarda o reemplaza el valor de una clave.
    pub fn set(&self, key: &str, value: &str) {
        self.inner
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
    }

    /// Devuelve una copia del valor para liberar el candado antes de responder.
    pub fn get(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().get(key).cloned()
    }

    /// Elimina la clave e indica si estaba almacenada.
    pub fn del(&self, key: &str) -> bool {
        self.inner.lock().unwrap().remove(key).is_some()
    }

    /// Incrementa bajo un único candado sin modificar valores inválidos.
    pub fn incr(&self, key: &str) -> Result<i64, &'static str> {
        let mut entries = self.inner.lock().unwrap();
        let value = match entries.get(key) {
            Some(value) => value
                .parse::<i64>()
                .ok()
                .and_then(|number| number.checked_add(1)),
            None => Some(1),
        }
        .ok_or("value is not an integer or out of range")?;

        entries.insert(key.to_string(), value.to_string());
        Ok(value)
    }
}
