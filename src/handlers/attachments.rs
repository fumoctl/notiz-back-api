use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::Response,
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    config::AppState,
    error::{AppError, AppResult},
    handlers::notes::get_note_access,
    models::{AttachmentBlob, AttachmentMetadata, AttachmentRow},
};

const ALLOWED_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
    "image/gif",
    "image/svg+xml",
];

/// POST /api/notes/{id}/attachments
/// Espera un `multipart/form-data` con un campo `file` (o `image`)
pub async fn upload_attachment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<AttachmentMetadata>)> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    if access_role != "owner" && access_role != "edit" {
        return Err(AppError::Forbidden(
            "No tienes permiso para adjuntar imágenes en esta nota".to_string(),
        ));
    }

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Error al leer multipart: {e}")))?
    {
        let field_name = field.name().unwrap_or("");
        if field_name != "file" && field_name != "image" {
            continue;
        }

        let filename = field
            .file_name()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "image.png".to_string());

        let content_type = field
            .content_type()
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        if !ALLOWED_IMAGE_TYPES.contains(&content_type.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Tipo de archivo no permitido ({content_type}). Solo se permiten imágenes (PNG, JPEG, WEBP, GIF, SVG)."
            )));
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("Error al leer los bytes de la imagen: {e}")))?;

        if data.is_empty() {
            return Err(AppError::BadRequest("El archivo está vacío".to_string()));
        }

        if data.len() > state.config.max_image_size_bytes {
            return Err(AppError::BadRequest(format!(
                "La imagen supera el tamaño máximo permitido de {} bytes",
                state.config.max_image_size_bytes
            )));
        }

        let row: AttachmentRow = sqlx::query_as(
            r#"
            INSERT INTO note_attachments (note_id, uploader_id, filename, content_type, size_bytes, data)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, note_id, uploader_id, filename, content_type, size_bytes, created_at
            "#,
        )
        .bind(note_id)
        .bind(auth_user.user_id)
        .bind(&filename)
        .bind(&content_type)
        .bind(data.len() as i32)
        .bind(data.as_ref())
        .fetch_one(&state.db)
        .await?;

        return Ok((StatusCode::CREATED, Json(row.into())));
    }

    Err(AppError::BadRequest(
        "No se encontró el campo 'file' o 'image' en el formulario multipart".to_string(),
    ))
}

/// GET /api/notes/{id}/attachments
/// Lista los metadatos de las imágenes adjuntas de una nota (sin descargar el BLOB)
pub async fn list_attachments(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
) -> AppResult<Json<Vec<AttachmentMetadata>>> {
    let _access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada o sin acceso".to_string()))?;

    let rows: Vec<AttachmentRow> = sqlx::query_as(
        r#"
        SELECT id, note_id, uploader_id, filename, content_type, size_bytes, created_at
        FROM note_attachments
        WHERE note_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(note_id)
    .fetch_all(&state.db)
    .await?;

    let metadata = rows.into_iter().map(AttachmentMetadata::from).collect();
    Ok(Json(metadata))
}

/// GET /api/notes/{id}/attachments/{attachment_id}
/// Devuelve el binario BLOB de la imagen con su Content-Type correspondiente
pub async fn get_attachment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((note_id, attachment_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    let _access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada o sin acceso".to_string()))?;

    let blob: Option<AttachmentBlob> = sqlx::query_as(
        r#"
        SELECT filename, content_type, data
        FROM note_attachments
        WHERE id = $1 AND note_id = $2
        "#,
    )
    .bind(attachment_id)
    .bind(note_id)
    .fetch_optional(&state.db)
    .await?;

    let blob = blob.ok_or_else(|| AppError::NotFound("Imagen adjunta no encontrada".to_string()))?;

    let mut headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(&blob.content_type) {
        headers.insert(CONTENT_TYPE, ct);
    }
    let safe_filename = blob.filename.replace('"', "");
    if let Ok(cd) = HeaderValue::from_str(&format!("inline; filename=\"{safe_filename}\"")) {
        headers.insert(CONTENT_DISPOSITION, cd);
    }
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );

    let mut response = Response::new(Body::from(blob.data));
    *response.status_mut() = StatusCode::OK;
    *response.headers_mut() = headers;

    Ok(response)
}

/// DELETE /api/notes/{id}/attachments/{attachment_id}
pub async fn delete_attachment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((note_id, attachment_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    if access_role != "owner" && access_role != "edit" {
        return Err(AppError::Forbidden(
            "No tienes permiso para eliminar imágenes de esta nota".to_string(),
        ));
    }

    let result = sqlx::query("DELETE FROM note_attachments WHERE id = $1 AND note_id = $2")
        .bind(attachment_id)
        .bind(note_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Imagen adjunta no encontrada".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

