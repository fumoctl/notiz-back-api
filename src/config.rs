use std::env;
use std::sync::Arc;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub google_client_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub max_image_size_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "La variable de entorno DATABASE_URL es obligatoria".to_string())?;

        let jwt_secret = env::var("JWT_SECRET")
            .map_err(|_| "La variable de entorno JWT_SECRET es obligatoria".to_string())?;

        if jwt_secret.len() < 16 {
            return Err("JWT_SECRET debe tener al menos 16 caracteres".to_string());
        }

        let jwt_expiration_hours = env::var("JWT_EXPIRATION_HOURS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(24 * 7); // 7 días por defecto

        let google_client_id = env::var("GOOGLE_CLIENT_ID")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(3000);

        let max_image_size_bytes = env::var("MAX_IMAGE_SIZE_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10 * 1024 * 1024); // 10 MB por defecto

        Ok(Self {
            database_url,
            jwt_secret,
            jwt_expiration_hours,
            google_client_id,
            host,
            port,
            max_image_size_bytes,
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
}

