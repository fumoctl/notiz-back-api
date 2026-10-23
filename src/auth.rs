use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::AppState,
    error::{AppError, AppResult},
    models::{
        AuthResponse, GoogleAuthRequest, LoginRequest, RegisterRequest, User, UserResponse,
    },
};

// ============================================================================
// JWT Claims y Extractor de Autenticación
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub username: String,
    pub iat: usize,
    pub exp: usize,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub username: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                AppError::Unauthorized("Falta el encabezado Authorization".to_string())
            })?;

        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            AppError::Unauthorized("El formato del token debe ser 'Bearer <token>'".to_string())
        })?;

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| AppError::Unauthorized("Token inválido o expirado".to_string()))?;

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            username: token_data.claims.username,
        })
    }
}

fn create_jwt(user: &User, state: &AppState) -> AppResult<String> {
    let now = Utc::now();
    let exp = now + Duration::hours(state.config.jwt_expiration_hours);

    let claims = Claims {
        sub: user.id,
        username: user.username.clone(),
        iat: now.timestamp() as usize,
        exp: exp.timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("No se pudo firmar el JWT: {e}")))
}

// ============================================================================
// Hashing de Contraseñas con Argon2id (en hilo bloqueante dedicado)
// ============================================================================

async fn hash_password(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(format!("Error al generar hash de contraseña: {e}")))
    })
    .await
    .map_err(|e| AppError::Internal(format!("Error en tarea de hashing: {e}")))?
}

async fn verify_password(password: String, password_hash: String) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || {
        let parsed_hash = PasswordHash::new(&password_hash)
            .map_err(|e| AppError::Internal(format!("Hash almacenado inválido: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Error en tarea de verificación: {e}")))?
}

// ============================================================================
// Handlers de Autenticación
// ============================================================================

/// POST /api/auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> AppResult<(StatusCode, Json<AuthResponse>)> {
    let username = payload.username.trim().to_string();
    let email = payload.email.trim().to_lowercase();

    if username.len() < 3 || username.len() > 64 {
        return Err(AppError::BadRequest(
            "El nombre de usuario debe tener entre 3 y 64 caracteres".to_string(),
        ));
    }
    if !email.contains('@') || email.len() > 255 {
        return Err(AppError::BadRequest(
            "El correo electrónico no es válido".to_string(),
        ));
    }
    if payload.password.len() < 8 {
        return Err(AppError::BadRequest(
            "La contraseña debe tener al menos 8 caracteres".to_string(),
        ));
    }

    // Verificar si ya existe el usuario o correo (case-insensitive)
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM users WHERE LOWER(username) = LOWER($1) OR LOWER(email) = LOWER($2) LIMIT 1",
    )
    .bind(&username)
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "El nombre de usuario o correo electrónico ya está registrado".to_string(),
        ));
    }

    let password_hash = hash_password(payload.password).await?;

    let user: User = sqlx::query_as(
        r#"
        INSERT INTO users (username, email, password_hash)
        VALUES ($1, $2, $3)
        RETURNING id, username, email, password_hash, google_sub, created_at
        "#,
    )
    .bind(&username)
    .bind(&email)
    .bind(&password_hash)
    .fetch_one(&state.db)
    .await?;

    let token = create_jwt(&user, &state)?;

    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            token,
            user: user.into(),
        }),
    ))
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let identifier = payload.identifier.trim();

    let user: Option<User> = sqlx::query_as(
        r#"
        SELECT id, username, email, password_hash, google_sub, created_at
        FROM users
        WHERE LOWER(username) = LOWER($1) OR LOWER(email) = LOWER($1)
        LIMIT 1
        "#,
    )
    .bind(identifier)
    .fetch_optional(&state.db)
    .await?;

    let user = user.ok_or_else(|| {
        AppError::Unauthorized("Credenciales inválidas".to_string())
    })?;

    let stored_hash = user.password_hash.clone().ok_or_else(|| {
        AppError::Unauthorized(
            "Esta cuenta fue creada con Google. Inicia sesión con Google.".to_string(),
        )
    })?;

    let is_valid = verify_password(payload.password, stored_hash).await?;
    if !is_valid {
        return Err(AppError::Unauthorized("Credenciales inválidas".to_string()));
    }

    let token = create_jwt(&user, &state)?;

    Ok(Json(AuthResponse {
        token,
        user: user.into(),
    }))
}

#[derive(Debug, Deserialize)]
struct GoogleTokenInfo {
    sub: String,
    email: String,
    email_verified: Option<String>,
    name: Option<String>,
    aud: String,
}

/// POST /api/auth/google
pub async fn google_login(
    State(state): State<AppState>,
    Json(payload): Json<GoogleAuthRequest>,
) -> AppResult<Json<AuthResponse>> {
    let expected_client_id = state.config.google_client_id.as_ref().ok_or_else(|| {
        AppError::BadRequest(
            "El inicio de sesión con Google no está configurado (GOOGLE_CLIENT_ID)".to_string(),
        )
    })?;

    // Validar el id_token contra el endpoint oficial de Google
    let resp = state
        .http_client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&[("id_token", payload.id_token.trim())])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Error al contactar con Google OAuth: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Unauthorized(
            "El token de Google es inválido o ha expirado".to_string(),
        ));
    }

    let token_info: GoogleTokenInfo = resp
        .json()
        .await
        .map_err(|_| AppError::Unauthorized("Respuesta inválida de Google OAuth".to_string()))?;

    if &token_info.aud != expected_client_id {
        return Err(AppError::Unauthorized(
            "El token de Google no pertenece a esta aplicación (aud mismatch)".to_string(),
        ));
    }

    if token_info.email_verified.as_deref() != Some("true") {
        return Err(AppError::Unauthorized(
            "El correo de la cuenta de Google no está verificado".to_string(),
        ));
    }

    let email = token_info.email.trim().to_lowercase();

    // Buscar si ya existe por google_sub o por email
    let existing_user: Option<User> = sqlx::query_as(
        r#"
        SELECT id, username, email, password_hash, google_sub, created_at
        FROM users
        WHERE google_sub = $1 OR LOWER(email) = LOWER($2)
        LIMIT 1
        "#,
    )
    .bind(&token_info.sub)
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    let user = match existing_user {
        Some(u) => {
            if u.google_sub.is_none() {
                sqlx::query_as::<_, User>(
                    r#"
                    UPDATE users
                    SET google_sub = $1
                    WHERE id = $2
                    RETURNING id, username, email, password_hash, google_sub, created_at
                    "#,
                )
                .bind(&token_info.sub)
                .bind(u.id)
                .fetch_one(&state.db)
                .await?
            } else {
                u
            }
        }
        None => {
            // Generar un username único a partir del nombre o prefijo del email
            let base_username = token_info
                .name
                .as_deref()
                .unwrap_or_else(|| email.split('@').next().unwrap_or("user"))
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>();

            let prefix = if base_username.len() >= 3 {
                base_username
            } else {
                "user".to_string()
            };

            let short_suffix = &Uuid::new_v4().simple().to_string()[..6];
            let username = format!("{}_{}", &prefix[..prefix.len().min(48)], short_suffix);

            sqlx::query_as::<_, User>(
                r#"
                INSERT INTO users (username, email, google_sub)
                VALUES ($1, $2, $3)
                RETURNING id, username, email, password_hash, google_sub, created_at
                "#,
            )
            .bind(&username)
            .bind(&email)
            .bind(&token_info.sub)
            .fetch_one(&state.db)
            .await?
        }
    };

    let token = create_jwt(&user, &state)?;

    Ok(Json(AuthResponse {
        token,
        user: user.into(),
    }))
}

/// GET /api/auth/me
pub async fn me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserResponse>> {
    let user: User = sqlx::query_as(
        r#"
        SELECT id, username, email, password_hash, google_sub, created_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(auth_user.user_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(user.into()))
}

