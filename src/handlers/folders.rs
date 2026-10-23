use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    config::AppState,
    error::{AppError, AppResult},
    models::{CreateFolderRequest, Folder, UpdateFolderRequest},
};

/// GET /api/folders
pub async fn list_folders(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<Vec<Folder>>> {
    let folders: Vec<Folder> = sqlx::query_as(
        r#"
        SELECT id, user_id, name, created_at
        FROM folders
        WHERE user_id = $1
        ORDER BY name ASC
        "#,
    )
    .bind(auth_user.user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(folders))
}

/// POST /api/folders
pub async fn create_folder(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateFolderRequest>,
) -> AppResult<(StatusCode, Json<Folder>)> {
    let name = payload.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(AppError::BadRequest(
            "El nombre de la carpeta debe tener entre 1 y 120 caracteres".to_string(),
        ));
    }

    let folder: Folder = sqlx::query_as(
        r#"
        INSERT INTO folders (user_id, name)
        VALUES ($1, $2)
        RETURNING id, user_id, name, created_at
        "#,
    )
    .bind(auth_user.user_id)
    .bind(name)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(folder)))
}

/// PUT /api/folders/{id}
pub async fn update_folder(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(folder_id): Path<Uuid>,
    Json(payload): Json<UpdateFolderRequest>,
) -> AppResult<Json<Folder>> {
    let name = payload.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(AppError::BadRequest(
            "El nombre de la carpeta debe tener entre 1 y 120 caracteres".to_string(),
        ));
    }

    let folder: Option<Folder> = sqlx::query_as(
        r#"
        UPDATE folders
        SET name = $1
        WHERE id = $2 AND user_id = $3
        RETURNING id, user_id, name, created_at
        "#,
    )
    .bind(name)
    .bind(folder_id)
    .bind(auth_user.user_id)
    .fetch_optional(&state.db)
    .await?;

    let folder = folder.ok_or_else(|| {
        AppError::NotFound("Carpeta no encontrada o sin permisos".to_string())
    })?;

    Ok(Json(folder))
}

/// DELETE /api/folders/{id}
pub async fn delete_folder(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(folder_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM folders WHERE id = $1 AND user_id = $2")
        .bind(folder_id)
        .bind(auth_user.user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "Carpeta no encontrada o sin permisos".to_string(),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}

