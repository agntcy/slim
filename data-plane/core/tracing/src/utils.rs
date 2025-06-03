use once_cell::sync::Lazy;
use uuid::Uuid;

pub static INSTANCE_ID: Lazy<String> =
    Lazy::new(|| std::env::var("SLIM_INSTANCE_ID").unwrap_or_else(|_| Uuid::new_v4().to_string()));
