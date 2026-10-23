use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    config::AppState,
    error::{AppError, AppResult},
    models::{
        CreateNoteRequest, ListNotesQuery, NoteResponse, NoteShareInfo, ShareNoteRequest,
        UpdateNoteRequest,
    },
};

/// Devuelve el nivel de acceso del usuario sobre una nota:
/// - `Some("owner")` si el usuario es el creador
/// - `Some("edit")` o `Some("read")` si fue compartida con él
/// - `None` si la nota no existe o el usuario no tiene acceso
pub async fn get_note_access(
    db: &PgPool,
    note_id: Uuid,
    user_id: Uuid,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT
            CASE
                WHEN n.owner_id = $2 THEN 'owner'
                ELSE ns.permission
            END AS access_role
        FROM notes n
        LEFT JOIN note_shares ns
            ON ns.note_id = n.id AND ns.shared_with_user_id = $2
        WHERE n.id = $1
          AND (n.owner_id = $2 OR ns.shared_with_user_id IS NOT NULL)
        "#,
    )
    .bind(note_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|(role,)| role))
}

async fn ensure_folder_belongs_to_user(
    db: &PgPool,
    folder_id: Uuid,
    user_id: Uuid,
) -> AppResult<()> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM folders WHERE id = $1 AND user_id = $2")
            .bind(folder_id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;

    if exists.is_none() {
        return Err(AppError::BadRequest(
            "La carpeta indicada no existe o no te pertenece".to_string(),
        ));
    }
    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

/// GET /api/notes
pub async fn list_notes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListNotesQuery>,
) -> AppResult<Json<Vec<NoteResponse>>> {
    let shared_only = query.shared_only.unwrap_or(false);
    let normalized_tag = query.tag.map(|t| t.trim().to_lowercase());

    let notes: Vec<NoteResponse> = sqlx::query_as(
        r#"
        SELECT
            n.id,
            n.owner_id,
            u.username AS owner_username,
            n.folder_id,
            n.title,
            n.body,
            n.tags,
            CASE
                WHEN n.owner_id = $1 THEN 'owner'
                ELSE ns.permission
            END AS access_role,
            n.created_at,
            n.updated_at
        FROM notes n
        INNER JOIN users u ON u.id = n.owner_id
        LEFT JOIN note_shares ns
            ON ns.note_id = n.id AND ns.shared_with_user_id = $1
        WHERE (
            ($2 = false AND (n.owner_id = $1 OR ns.shared_with_user_id = $1))
            OR ($2 = true AND ns.shared_with_user_id = $1)
        )
          AND ($3::UUID IS NULL OR n.folder_id = $3)
          AND ($4::TEXT IS NULL OR $4 = ANY(n.tags))
        ORDER BY n.updated_at DESC
        "#,
    )
    .bind(auth_user.user_id)
    .bind(shared_only)
    .bind(query.folder_id)
    .bind(normalized_tag)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(notes))
}

/// POST /api/notes
pub async fn create_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateNoteRequest>,
) -> AppResult<(StatusCode, Json<NoteResponse>)> {
    let title = payload.title.trim();
    if title.is_empty() || title.len() > 255 {
        return Err(AppError::BadRequest(
            "El título debe tener entre 1 y 255 caracteres".to_string(),
        ));
    }

    if let Some(folder_id) = payload.folder_id {
        ensure_folder_belongs_to_user(&state.db, folder_id, auth_user.user_id).await?;
    }

    let tags = normalize_tags(payload.tags);

    let note: NoteResponse = sqlx::query_as(
        r#"
        INSERT INTO notes (owner_id, folder_id, title, body, tags)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING
            id,
            owner_id,
            $6 AS owner_username,
            folder_id,
            title,
            body,
            tags,
            'owner' AS access_role,
            created_at,
            updated_at
        "#,
    )
    .bind(auth_user.user_id)
    .bind(payload.folder_id)
    .bind(title)
    .bind(&payload.body)
    .bind(&tags)
    .bind(&auth_user.username)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(note)))
}

/// GET /api/notes/{id}
pub async fn get_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
) -> AppResult<Json<NoteResponse>> {
    let note: Option<NoteResponse> = sqlx::query_as(
        r#"
        SELECT
            n.id,
            n.owner_id,
            u.username AS owner_username,
            n.folder_id,
            n.title,
            n.body,
            n.tags,
            CASE
                WHEN n.owner_id = $2 THEN 'owner'
                ELSE ns.permission
            END AS access_role,
            n.created_at,
            n.updated_at
        FROM notes n
        INNER JOIN users u ON u.id = n.owner_id
        LEFT JOIN note_shares ns
            ON ns.note_id = n.id AND ns.shared_with_user_id = $2
        WHERE n.id = $1
          AND (n.owner_id = $2 OR ns.shared_with_user_id = $2)
        "#,
    )
    .bind(note_id)
    .bind(auth_user.user_id)
    .fetch_optional(&state.db)
    .await?;

    let note = note.ok_or_else(|| {
        AppError::NotFound("Nota no encontrada o sin permisos de lectura".to_string())
    })?;

    Ok(Json(note))
}

/// PUT /api/notes/{id}
pub async fn update_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
    Json(payload): Json<UpdateNoteRequest>,
) -> AppResult<Json<NoteResponse>> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    if access_role != "owner" && access_role != "edit" {
        return Err(AppError::Forbidden(
            "Solo tienes permiso de lectura sobre esta nota".to_string(),
        ));
    }

    // Obtener valores actuales para aplicar actualización parcial
    let current = get_note(State(state.clone()), auth_user.clone(), Path(note_id)).await?.0;

    let new_title = match payload.title {
        Some(t) => {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() || trimmed.len() > 255 {
                return Err(AppError::BadRequest(
                    "El título debe tener entre 1 y 255 caracteres".to_string(),
                ));
            }
            trimmed
        }
        None => current.title,
    };

    let new_body = payload.body.unwrap_or(current.body);
    let new_tags = payload.tags.map(normalize_tags).unwrap_or(current.tags);

    let new_folder_id = match payload.folder_id {
        Some(folder_opt) => {
            if access_role != "owner" {
                return Err(AppError::Forbidden(
                    "Solo el propietario de la nota puede cambiar su carpeta".to_string(),
                ));
            }
            if let Some(fid) = folder_opt {
                ensure_folder_belongs_to_user(&state.db, fid, auth_user.user_id).await?;
            }
            folder_opt
        }
        None => current.folder_id,
    };

    let updated: NoteResponse = sqlx::query_as(
        r#"
        UPDATE notes
        SET title = $1,
            body = $2,
            tags = $3,
            folder_id = $4,
            updated_at = NOW()
        WHERE id = $5
        RETURNING
            id,
            owner_id,
            $6 AS owner_username,
            folder_id,
            title,
            body,
            tags,
            $7 AS access_role,
            created_at,
            updated_at
        "#,
    )
    .bind(&new_title)
    .bind(&new_body)
    .bind(&new_tags)
    .bind(new_folder_id)
    .bind(note_id)
    .bind(&current.owner_username)
    .bind(&access_role)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(updated))
}

/// DELETE /api/notes/{id}
pub async fn delete_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM notes WHERE id = $1 AND owner_id = $2")
        .bind(note_id)
        .bind(auth_user.user_id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "Nota no encontrada o no eres el propietario".to_string(),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Compartir Notas de forma Privada (por username o email)
// ============================================================================

/// POST /api/notes/{id}/shares
pub async fn share_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
    Json(payload): Json<ShareNoteRequest>,
) -> AppResult<(StatusCode, Json<NoteShareInfo>)> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    if access_role != "owner" {
        return Err(AppError::Forbidden(
            "Solo el propietario puede compartir esta nota".to_string(),
        ));
    }

    let recipient = payload.recipient.trim();
    if recipient.is_empty() {
        return Err(AppError::BadRequest(
            "Debes especificar el nombre de usuario o correo del destinatario".to_string(),
        ));
    }

    let target_user: Option<(Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT id, username, email
        FROM users
        WHERE LOWER(username) = LOWER($1) OR LOWER(email) = LOWER($1)
        LIMIT 1
        "#,
    )
    .bind(recipient)
    .fetch_optional(&state.db)
    .await?;

    let (target_user_id, target_username, target_email) = target_user.ok_or_else(|| {
        AppError::NotFound(format!(
            "No se encontró ningún usuario con el identificador '{recipient}'"
        ))
    })?;

    if target_user_id == auth_user.user_id {
        return Err(AppError::BadRequest(
            "No puedes compartir una nota contigo mismo".to_string(),
        ));
    }

    let share_info: NoteShareInfo = sqlx::query_as(
        r#"
        INSERT INTO note_shares (note_id, shared_with_user_id, permission)
        VALUES ($1, $2, $3)
        ON CONFLICT (note_id, shared_with_user_id)
        DO UPDATE SET permission = EXCLUDED.permission
        RETURNING
            note_id,
            shared_with_user_id,
            $4 AS username,
            $5 AS email,
            permission,
            created_at
        "#,
    )
    .bind(note_id)
    .bind(target_user_id)
    .bind(payload.permission.as_str())
    .bind(target_username)
    .bind(target_email)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(share_info)))
}

/// GET /api/notes/{id}/shares
pub async fn list_note_shares(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(note_id): Path<Uuid>,
) -> AppResult<Json<Vec<NoteShareInfo>>> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    if access_role != "owner" {
        return Err(AppError::Forbidden(
            "Solo el propietario puede ver la lista de usuarios con acceso".to_string(),
        ));
    }

    let shares: Vec<NoteShareInfo> = sqlx::query_as(
        r#"
        SELECT
            ns.note_id,
            ns.shared_with_user_id,
            u.username,
            u.email,
            ns.permission,
            ns.created_at
        FROM note_shares ns
        INNER JOIN users u ON u.id = ns.shared_with_user_id
        WHERE ns.note_id = $1
        ORDER BY ns.created_at DESC
        "#,
    )
    .bind(note_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(shares))
}

/// DELETE /api/notes/{id}/shares/{target_user_id}
pub async fn unshare_note(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((note_id, target_user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    let access_role = get_note_access(&state.db, note_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Nota no encontrada".to_string()))?;

    // Permite revocar el acceso tanto al propietario como al propio receptor si desea quitarse la nota
    if access_role != "owner" && auth_user.user_id != target_user_id {
        return Err(AppError::Forbidden(
            "No tienes permiso para revocar este acceso compartido".to_string(),
        ));
    }

    let result = sqlx::query(
        "DELETE FROM note_shares WHERE note_id = $1 AND shared_with_user_id = $2",
    )
    .bind(note_id)
    .bind(target_user_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "La nota no estaba compartida con ese usuario".to_string(),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}

