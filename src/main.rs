mod auth;
mod config;
mod error;
mod handlers;
mod models;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::{AppState, Config},
    handlers::{attachments, folders, notes},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "notiz_back_api=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(Config::from_env()?);

    tracing::info!("Conectando a PostgreSQL / YugabyteDB...");
    let db = PgPoolOptions::new()
        .max_connections(15)
        .connect(&config.database_url)
        .await?;

    tracing::info!("Ejecutando migraciones de base de datos...");
    sqlx::migrate!("./migrations").run(&db).await?;

    let state = AppState {
        db,
        config: config.clone(),
        http_client: reqwest::Client::new(),
    };

    let app = build_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Servidor REST API escuchando en http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_router(state: AppState) -> Router {
    let max_body = state.config.max_image_size_bytes + (1024 * 1024);

    Router::new()
        .route("/api/health", get(health_check))
        // Autenticación
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/google", post(auth::google_login))
        .route("/api/auth/me", get(auth::me))
        // Carpetas
        .route(
            "/api/folders",
            get(folders::list_folders).post(folders::create_folder),
        )
        .route(
            "/api/folders/{id}",
            put(folders::update_folder).delete(folders::delete_folder),
        )
        // Notas
        .route(
            "/api/notes",
            get(notes::list_notes).post(notes::create_note),
        )
        .route(
            "/api/notes/{id}",
            get(notes::get_note)
                .put(notes::update_note)
                .delete(notes::delete_note),
        )
        // Compartir notas de forma privada
        .route(
            "/api/notes/{id}/shares",
            get(notes::list_note_shares).post(notes::share_note),
        )
        .route(
            "/api/notes/{id}/shares/{target_user_id}",
            delete(notes::unshare_note),
        )
        // Imágenes adjuntas (BLOBs en base de datos)
        .route(
            "/api/notes/{id}/attachments",
            get(attachments::list_attachments).post(attachments::upload_attachment),
        )
        .route(
            "/api/notes/{id}/attachments/{attachment_id}",
            get(attachments::get_attachment).delete(attachments::delete_attachment),
        )
        .layer(DefaultBodyLimit::max(max_body))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
