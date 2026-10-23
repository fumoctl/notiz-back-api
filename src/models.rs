use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ============================================================================
// Usuarios y Autenticación
// ============================================================================

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub google_sub: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            created_at: user.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Puede ser el nombre de usuario o el correo electrónico
    pub identifier: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleAuthRequest {
    /// ID Token emitido por Google Sign-In en el frontend
    pub id_token: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserResponse,
}

// ============================================================================
// Carpetas
// ============================================================================

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Folder {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFolderRequest {
    pub name: String,
}

// ============================================================================
// Notas y Compartición
// ============================================================================

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NoteResponse {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_username: String,
    pub folder_id: Option<Uuid>,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    /// Rol del usuario actual sobre la nota: "owner", "edit" o "read"
    pub access_role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNoteRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub folder_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNoteRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Si se envía `Some(Some(uuid))` asigna carpeta; `Some(None)` la saca de la carpeta; `None` no modifica
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub folder_id: Option<Option<Uuid>>,
}

fn deserialize_optional_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Deserialize, Default)]
pub struct ListNotesQuery {
    pub folder_id: Option<Uuid>,
    pub tag: Option<String>,
    /// Si es `true`, devuelve únicamente las notas que otros usuarios compartieron conmigo
    pub shared_only: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SharePermission {
    Read,
    Edit,
}

impl SharePermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            SharePermission::Read => "read",
            SharePermission::Edit => "edit",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ShareNoteRequest {
    /// Nombre de usuario o correo electrónico del usuario receptor
    pub recipient: String,
    #[serde(default = "default_share_permission")]
    pub permission: SharePermission,
}

fn default_share_permission() -> SharePermission {
    SharePermission::Read
}

#[derive(Debug, Serialize, FromRow)]
pub struct NoteShareInfo {
    pub note_id: Uuid,
    pub shared_with_user_id: Uuid,
    pub username: String,
    pub email: String,
    pub permission: String,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Imágenes Adjuntas (BLOBs en BD)
// ============================================================================

#[derive(Debug, Clone, FromRow)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub note_id: Uuid,
    pub uploader_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AttachmentMetadata {
    pub id: Uuid,
    pub note_id: Uuid,
    pub uploader_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i32,
    pub url: String,
    /// Snippet Markdown listo para insertar en el cuerpo de la nota
    pub markdown: String,
    pub created_at: DateTime<Utc>,
}

impl From<AttachmentRow> for AttachmentMetadata {
    fn from(row: AttachmentRow) -> Self {
        let url = format!("/api/notes/{}/attachments/{}", row.note_id, row.id);
        let markdown = format!("![{}]({})", row.filename, url);
        Self {
            id: row.id,
            note_id: row.note_id,
            uploader_id: row.uploader_id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            url,
            markdown,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, FromRow)]
pub struct AttachmentBlob {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

