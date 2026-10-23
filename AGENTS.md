# AGENTS.md — Guía de Arquitectura, Mantenimiento y Especificación de Notiz Back API

Este documento es la referencia principal para agentes de IA y desarrolladores que trabajen en el desarrollo, mantenimiento o integración (frontend/clientes) de `notiz-back-api`.

---

## 1. Visión General y Stack Tecnológico

`notiz-back-api` es el backend REST de una aplicación de notas en formato **Markdown** con soporte para etiquetas (`tags`), organización en carpetas (`folders`), adjuntos de imágenes almacenados como **BLOBs (`BYTEA`)** en base de datos y compartición privada entre usuarios con control de permisos (`read` / `edit`).

- **Lenguaje:** Rust (`edition = "2024"`)
- **Framework Web:** Axum `0.8` (`multipart`, `macros`) + Tower HTTP (`cors`, `trace`, `limit`)
- **Runtime Asíncrono:** Tokio `1.x` (`full`)
- **Base de Datos:** YugabyteDB (API YSQL en puerto `5433`) o PostgreSQL (puerto `5432`)
- **Driver / Migraciones:** SQLx `0.8` (`postgres`, `runtime-tokio`, `tls-rustls`, `migrate`)
- **Autenticación:**
  - Contraseñas locales con hash **Argon2id** (`argon2`)
  - Tokens **JWT** (`HS256` vía `jsonwebtoken`)
  - Login con **Google OAuth2 (`id_token`)** verificado contra `https://oauth2.googleapis.com/tokeninfo`
- **Entorno de Desarrollo:** Nix Flakes (`flake.nix` + `direnv`) y contenedor `podman-yugabyte`

---

## 2. Estructura del Proyecto

```text
notiz-back-api/
├── Cargo.toml                 # Dependencias y metadatos del crate
├── flake.nix                  # Entorno reproducible Nix (Rust, clippy, rust-analyzer, curl, httpie)
├── .env.example               # Plantilla de variables de entorno
├── migrations/
│   └── 0001_init.sql          # Esquema inicial compatible con YugabyteDB (YSQL) y PostgreSQL
├── scripts/
│   └── test_api.sh            # Suite de pruebas E2E automatizada con curl
└── src/
    ├── main.rs                # Punto de entrada, pool de BD, migraciones y definición del Router
    ├── config.rs              # Carga/validación de variables de entorno y definición de AppState
    ├── error.rs               # Enum AppError e implementación centralizada de IntoResponse
    ├── auth.rs                # Argon2id, generación/validación de JWT, extractor AuthUser y rutas /api/auth/*
    ├── models.rs              # Estructuras de base de datos (FromRow) y DTOs de entrada/salida (Serialize/Deserialize)
    └── handlers/
        ├── mod.rs             # Exportación de módulos de handlers
        ├── folders.rs         # CRUD de carpetas (/api/folders)
        ├── notes.rs           # CRUD de notas, filtros y compartición privada (/api/notes, /api/notes/{id}/shares)
        └── attachments.rs     # Subida multipart, listado, descarga binaria y borrado de imágenes BLOB
```

---

## 3. Modelo de Datos y Esquema SQL (YugabyteDB / PostgreSQL)

Todas las tablas utilizan **`UUID` (`gen_random_uuid()`)** como clave primaria (evita *hotspots* de escritura en tablets distribuidos de YugabyteDB) e incluyen **`created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`**.

### 3.1. Tabla `users`
| Columna | Tipo | Restricciones | Descripción |
| :--- | :--- | :--- | :--- |
| `id` | `UUID` | `PRIMARY KEY DEFAULT gen_random_uuid()` | Identificador único del usuario |
| `username` | `VARCHAR(64)` | `NOT NULL UNIQUE` | Nombre de usuario único (indexado también con `LOWER(username)`) |
| `email` | `VARCHAR(255)` | `NOT NULL UNIQUE` | Correo electrónico en minúsculas (`LOWER(email)`) |
| `password_hash` | `TEXT` | `NULL` | Hash PHC Argon2id (`NULL` si la cuenta se creó solo con Google) |
| `google_sub` | `TEXT` | `UNIQUE NULL` | Identificador `sub` único de la cuenta de Google |
| `created_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha de creación del usuario |

### 3.2. Tabla `folders`
| Columna | Tipo | Restricciones | Descripción |
| :--- | :--- | :--- | :--- |
| `id` | `UUID` | `PRIMARY KEY DEFAULT gen_random_uuid()` | Identificador de la carpeta |
| `user_id` | `UUID` | `NOT NULL REFERENCES users(id) ON DELETE CASCADE` | Propietario de la carpeta |
| `name` | `VARCHAR(120)` | `NOT NULL`, `UNIQUE(user_id, name)` | Nombre único de carpeta por usuario |
| `created_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha de creación de la carpeta |

### 3.3. Tabla `notes`
| Columna | Tipo | Restricciones | Descripción |
| :--- | :--- | :--- | :--- |
| `id` | `UUID` | `PRIMARY KEY DEFAULT gen_random_uuid()` | Identificador de la nota |
| `owner_id` | `UUID` | `NOT NULL REFERENCES users(id) ON DELETE CASCADE` | Creador/propietario de la nota |
| `folder_id` | `UUID` | `NULL REFERENCES folders(id) ON DELETE SET NULL` | Carpeta opcional (si se borra la carpeta, la nota pasa a `NULL`) |
| `title` | `VARCHAR(255)` | `NOT NULL` | Título de la nota |
| `body` | `TEXT` | `NOT NULL DEFAULT ''` | Cuerpo de la nota en formato Markdown |
| `tags` | `TEXT[]` | `NOT NULL DEFAULT '{}'` | Array nativo de etiquetas normalizadas (minúsculas, sin duplicados) |
| `created_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha de creación |
| `updated_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha de última modificación |

### 3.4. Tabla `note_shares`
| Columna | Tipo | Restricciones | Descripción |
| :--- | :--- | :--- | :--- |
| `note_id` | `UUID` | `REFERENCES notes(id) ON DELETE CASCADE` | Nota compartida (PK compuesta) |
| `shared_with_user_id` | `UUID` | `REFERENCES users(id) ON DELETE CASCADE` | Usuario receptor (PK compuesta) |
| `permission` | `VARCHAR(16)` | `CHECK (permission IN ('read', 'edit'))` | Nivel de acceso otorgado |
| `created_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha en que se compartió |

### 3.5. Tabla `note_attachments`
| Columna | Tipo | Restricciones | Descripción |
| :--- | :--- | :--- | :--- |
| `id` | `UUID` | `PRIMARY KEY DEFAULT gen_random_uuid()` | Identificador de la imagen adjunta |
| `note_id` | `UUID` | `NOT NULL REFERENCES notes(id) ON DELETE CASCADE` | Nota a la que pertenece |
| `uploader_id` | `UUID` | `NOT NULL REFERENCES users(id) ON DELETE CASCADE` | Usuario que subió la imagen |
| `filename` | `VARCHAR(255)` | `NOT NULL` | Nombre original del archivo |
| `content_type` | `VARCHAR(100)` | `NOT NULL` | MIME type (`image/png`, `image/jpeg`, `image/webp`, `image/gif`, `image/svg+xml`) |
| `size_bytes` | `INT` | `NOT NULL` | Tamaño en bytes |
| `data` | `BYTEA` | `NOT NULL` | **BLOB binario** de la imagen |
| `created_at` | `TIMESTAMPTZ` | `NOT NULL DEFAULT NOW()` | Fecha de subida |

---

## 4. Modelo de Seguridad y Permisos (Autorización)

### 4.1. Autenticación JWT (`AuthUser`)
Todas las rutas protegidas reciben el extractor `auth_user: AuthUser` definido en `src/auth.rs`.
- El cliente debe enviar el header HTTP:
  ```http
  Authorization: Bearer <jwt_token>
  ```
- Los claims del JWT contienen:
  - `sub`: `UUID` del usuario
  - `username`: nombre de usuario
  - `iat` y `exp`: timestamps UNIX

### 4.2. Matriz de Permisos sobre Notas y Adjuntos
La función `handlers::notes::get_note_access(&db, note_id, user_id)` determina el rol efectivo del usuario autenticado sobre cualquier nota (`"owner"`, `"edit"`, `"read"` o `None`):

| Acción | `owner` (Creador) | `edit` (Compartido) | `read` (Compartido) | Otros usuarios |
| :--- | :---: | :---: | :---: | :---: |
| Ver nota (`GET /api/notes/{id}`) | ✅ | ✅ | ✅ | ❌ (`404`) |
| Editar `title`, `body`, `tags` (`PUT /api/notes/{id}`) | ✅ | ✅ | ❌ (`403`) | ❌ (`404`) |
| Cambiar `folder_id` de la nota (`PUT /api/notes/{id}`) | ✅ | ❌ (`403`) | ❌ (`403`) | ❌ (`404`) |
| Eliminar nota (`DELETE /api/notes/{id}`) | ✅ | ❌ (`404`) | ❌ (`404`) | ❌ (`404`) |
| Compartir / listar accesos (`POST/GET /shares`) | ✅ | ❌ (`403`) | ❌ (`403`) | ❌ (`404`) |
| Quitar acceso compartido (`DELETE /shares/{uid}`) | ✅ (a cualquiera) | ✅ (solo a sí mismo) | ✅ (solo a sí mismo) | ❌ (`404`) |
| Ver / descargar imágenes BLOB (`GET /attachments/*`) | ✅ | ✅ | ✅ | ❌ (`404`) |
| Subir / borrar imágenes BLOB (`POST/DELETE /attachments`) | ✅ | ✅ | ❌ (`403`) | ❌ (`404`) |

---

## 5. Especificación Detallada de Endpoints REST

Formato de error estándar en toda la API:
```json
{
  "error": "Descripción legible del error"
}
```

---

### 5.1. Salud del Servidor

#### `GET /api/health`
- **Auth:** No requerida
- **Respuesta `200 OK`:**
  ```json
  { "status": "ok" }
  ```

---

### 5.2. Autenticación (`/api/auth`)

#### `POST /api/auth/register`
Registra un nuevo usuario local hasheando su contraseña con Argon2id.
- **Auth:** No requerida
- **Body (`application/json`):**
  ```json
  {
    "username": "fumoctl",
    "email": "fumo@example.com",
    "password": "contraseña_min_8_chars"
  }
  ```
- **Validaciones:**
  - `username`: entre 3 y 64 caracteres, único (case-insensitive).
  - `email`: formato válido con `@`, único (case-insensitive).
  - `password`: mínimo 8 caracteres.
- **Respuesta `201 Created`:**
  ```json
  {
    "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...",
    "user": {
      "id": "0916be94-de20-4273-b1ea-e4d7838713a6",
      "username": "fumoctl",
      "email": "fumo@example.com",
      "created_at": "2026-10-23T21:54:30.159206Z"
    }
  }
  ```
- **Errores:** `400 Bad Request`, `409 Conflict` (usuario o email duplicado).

#### `POST /api/auth/login`
Inicia sesión usando nombre de usuario **o** correo electrónico.
- **Auth:** No requerida
- **Body (`application/json`):**
  ```json
  {
    "identifier": "fumoctl_o_correo@example.com",
    "password": "contraseña_min_8_chars"
  }
  ```
- **Respuesta `200 OK`:** Mismo esquema `AuthResponse` (`token` + `user`).
- **Errores:** `401 Unauthorized`.

#### `POST /api/auth/google`
Inicia sesión o registra automáticamente al usuario validando un `id_token` emitido por Google Sign-In. Si ya existía una cuenta con ese correo, vincula su `google_sub`.
- **Auth:** No requerida (requiere tener `GOOGLE_CLIENT_ID` configurado en `.env`).
- **Body (`application/json`):**
  ```json
  {
    "id_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6..."
  }
  ```
- **Respuesta `200 OK`:** Mismo esquema `AuthResponse` (`token` + `user`).

#### `GET /api/auth/me`
Devuelve los datos del usuario actualmente autenticado.
- **Auth:** `Bearer <token>`
- **Respuesta `200 OK`:**
  ```json
  {
    "id": "0916be94-de20-4273-b1ea-e4d7838713a6",
    "username": "fumoctl",
    "email": "fumo@example.com",
    "created_at": "2026-10-23T21:54:30.159206Z"
  }
  ```

---

### 5.3. Carpetas (`/api/folders`)

#### `GET /api/folders`
Lista todas las carpetas pertenecientes al usuario autenticado ordenadas alfabéticamente.
- **Auth:** `Bearer <token>`
- **Respuesta `200 OK`:**
  ```json
  [
    {
      "id": "6a68cfe8-d8d8-40ac-87b2-afb2013430a0",
      "user_id": "0916be94-de20-4273-b1ea-e4d7838713a6",
      "name": "Proyectos Rust",
      "created_at": "2026-10-23T21:54:30.731559Z"
    }
  ]
  ```

#### `POST /api/folders`
Crea una carpeta para el usuario autenticado.
- **Auth:** `Bearer <token>`
- **Body (`application/json`):**
  ```json
  { "name": "Proyectos Rust" }
  ```
- **Respuesta `201 Created`:** Objeto `Folder`.
- **Errores:** `400 Bad Request` (nombre vacío o >120 chars), `409 Conflict` (ya tiene una carpeta con ese nombre).

#### `PUT /api/folders/{id}`
Renombra una carpeta del usuario.
- **Auth:** `Bearer <token>`
- **Body (`application/json`):**
  ```json
  { "name": "Nuevo Nombre" }
  ```
- **Respuesta `200 OK`:** Objeto `Folder` actualizado.

#### `DELETE /api/folders/{id}`
Elimina la carpeta. Las notas que estaban dentro **no se borran**: su campo `folder_id` pasa automáticamente a `null`.
- **Auth:** `Bearer <token>`
- **Respuesta:** `204 No Content`.

---

### 5.4. Notas (`/api/notes`)

#### `GET /api/notes`
Lista las notas creadas por el usuario y las que otros usuarios han compartido con él, ordenadas por `updated_at DESC`.
- **Auth:** `Bearer <token>`
- **Query Params (opcionales):**
  - `folder_id=<uuid>`: Filtra notas dentro de una carpeta específica.
  - `tag=<string>`: Filtra notas que contengan esa etiqueta (insensible a mayúsculas).
  - `shared_only=true`: Devuelve únicamente las notas que otros usuarios compartieron conmigo.
- **Respuesta `200 OK`:**
  ```json
  [
    {
      "id": "edd5cc5e-8883-4ade-b0af-656b56ca7439",
      "owner_id": "0916be94-de20-4273-b1ea-e4d7838713a6",
      "owner_username": "fumoctl",
      "folder_id": "6a68cfe8-d8d8-40ac-87b2-afb2013430a0",
      "title": "Arquitectura del Backend",
      "body": "# Notiz API\n\nNota en **Markdown**.",
      "tags": ["axum", "rust", "yugabytedb"],
      "access_role": "owner",
      "created_at": "2026-10-23T21:54:30.744357Z",
      "updated_at": "2026-10-23T21:54:30.744357Z"
    }
  ]
  ```

#### `POST /api/notes`
Crea una nueva nota en Markdown.
- **Auth:** `Bearer <token>`
- **Body (`application/json`):**
  ```json
  {
    "title": "Arquitectura del Backend",
    "body": "# Notiz API\n\nNota en **Markdown**.",
    "tags": ["Rust", "Axum"],
    "folder_id": "6a68cfe8-d8d8-40ac-87b2-afb2013430a0"
  }
  ```
  *(Nota: `body`, `tags` y `folder_id` son opcionales. Los `tags` se normalizan a minúsculas, sin espacios extremos ni duplicados).*
- **Respuesta `201 Created`:** Objeto `NoteResponse`.

#### `GET /api/notes/{id}`
Obtiene el detalle completo de una nota si el usuario es propietario o tiene acceso compartido.
- **Auth:** `Bearer <token>`
- **Respuesta `200 OK`:** Objeto `NoteResponse`.

#### `PUT /api/notes/{id}`
Actualiza parcialmente una nota. Todos los campos del JSON son opcionales:
- Si `folder_id` no se incluye en el JSON, se conserva la carpeta actual.
- Si se envía `"folder_id": null`, se saca la nota de su carpeta.
- Si se envía `"folder_id": "<uuid>"`, se mueve a esa carpeta (solo permitido para el `owner`).
- **Auth:** `Bearer <token>` (`owner` o permiso `edit`)
- **Body (`application/json`):**
  ```json
  {
    "title": "Nuevo título opcional",
    "body": "Nuevo cuerpo Markdown opcional",
    "tags": ["nuevo-tag"],
    "folder_id": null
  }
  ```
- **Respuesta `200 OK`:** Objeto `NoteResponse` con `updated_at` renovado.

#### `DELETE /api/notes/{id}`
Elimina permanentemente una nota junto con sus comparticiones y adjuntos BLOB (`ON DELETE CASCADE`). Solo permitido al propietario (`owner`).
- **Auth:** `Bearer <token>`
- **Respuesta:** `204 No Content`.

---

### 5.5. Compartición Privada de Notas (`/api/notes/{id}/shares`)

#### `POST /api/notes/{id}/shares`
Comparte una nota con otro usuario indicando su **nombre de usuario** o su **correo electrónico**. Si ya estaba compartida con ese usuario, actualiza su nivel de permiso (`read` o `edit`).
- **Auth:** `Bearer <token>` (solo `owner`)
- **Body (`application/json`):**
  ```json
  {
    "recipient": "bob_username_o_correo@example.com",
    "permission": "edit"
  }
  ```
  *(`permission` acepta `"read"` o `"edit"`; por defecto es `"read"`).*
- **Respuesta `201 Created`:**
  ```json
  {
    "note_id": "edd5cc5e-8883-4ade-b0af-656b56ca7439",
    "shared_with_user_id": "fc06c2d8-bd98-400f-844f-4c08861b2db0",
    "username": "bob",
    "email": "bob@example.com",
    "permission": "edit",
    "created_at": "2026-10-23T21:54:30.775749Z"
  }
  ```

#### `GET /api/notes/{id}/shares`
Lista todos los usuarios con los que se ha compartido la nota.
- **Auth:** `Bearer <token>` (solo `owner`)
- **Respuesta `200 OK`:** Array de `NoteShareInfo`.

#### `DELETE /api/notes/{id}/shares/{target_user_id}`
Revoca el acceso compartido. Puede ejecutarlo el propietario de la nota para quitar a cualquier usuario, o el propio usuario receptor (`target_user_id == auth_user.user_id`) para abandonar la nota compartida.
- **Auth:** `Bearer <token>`
- **Respuesta:** `204 No Content`.

---

### 5.6. Imágenes Adjuntas como BLOBs (`/api/notes/{id}/attachments`)

#### `POST /api/notes/{id}/attachments`
Sube una imagen mediante `multipart/form-data` y almacena sus bytes crudos en la columna `BYTEA` (`data`) de la tabla `note_attachments`.
- **Auth:** `Bearer <token>` (`owner` o permiso `edit`)
- **Content-Type:** `multipart/form-data`
- **Campo del formulario:** `file` (también acepta `image`)
- **Tipos MIME permitidos:** `image/png`, `image/jpeg`, `image/jpg`, `image/webp`, `image/gif`, `image/svg+xml`
- **Límite de tamaño:** Configurable vía `MAX_IMAGE_SIZE_BYTES` (por defecto `10 MB`).
- **Respuesta `201 Created`:**
  ```json
  {
    "id": "e0602db6-5e64-4187-8e8e-f9729901afe4",
    "note_id": "edd5cc5e-8883-4ade-b0af-656b56ca7439",
    "uploader_id": "0916be94-de20-4273-b1ea-e4d7838713a6",
    "filename": "diagrama.png",
    "content_type": "image/png",
    "size_bytes": 67,
    "url": "/api/notes/edd5cc5e-8883-4ade-b0af-656b56ca7439/attachments/e0602db6-5e64-4187-8e8e-f9729901afe4",
    "markdown": "![diagrama.png](/api/notes/edd5cc5e-8883-4ade-b0af-656b56ca7439/attachments/e0602db6-5e64-4187-8e8e-f9729901afe4)",
    "created_at": "2026-10-23T21:54:30.756118Z"
  }
  ```

#### `GET /api/notes/{id}/attachments`
Lista únicamente los metadatos de las imágenes adjuntas de la nota (sin consultar la columna `data` `BYTEA` para mantener la respuesta ligera).
- **Auth:** `Bearer <token>` (`owner`, `edit` o `read`)
- **Respuesta `200 OK`:** Array de `AttachmentMetadata`.

#### `GET /api/notes/{id}/attachments/{attachment_id}`
Descarga/sirve los bytes binarios del BLOB (`BYTEA`) con las cabeceras HTTP correspondientes:
- `Content-Type: <image/mime>`
- `Content-Disposition: inline; filename="<filename>"`
- `Cache-Control: private, max-age=86400`
- **Auth:** `Bearer <token>` (`owner`, `edit` o `read`)
- **Respuesta `200 OK`:** Cuerpo binario de la imagen.

#### `DELETE /api/notes/{id}/attachments/{attachment_id}`
Elimina la imagen adjunta de la base de datos.
- **Auth:** `Bearer <token>` (`owner` o `edit`)
- **Respuesta:** `204 No Content`.

---

## 6. Reglas de Desarrollo y Mantenimiento para Agentes

1. **Calidad y Compilación Limpia:**
   Antes de dar por finalizada cualquier modificación en código Rust, ejecuta siempre:
   ```bash
   cargo check
   cargo clippy -- -D warnings
   ```
   El proyecto debe compilar con **0 warnings**.

2. **Operaciones CPU-intensivas en Tokio:**
   Nunca ejecutes hashing de contraseñas (`Argon2`) ni tareas síncronas pesadas directamente en el hilo del executor de Tokio; usa siempre `tokio::task::spawn_blocking` (como en `src/auth.rs`).

3. **Manejo de Errores:**
   - Todos los handlers deben retornar `AppResult<T>` (`Result<T, AppError>`).
   - Evita `.unwrap()` o `.expect()` dentro de handlers HTTP; propaga los errores con `?` o conviértelos explícitamente en la variante adecuada de `AppError` (`BadRequest`, `Unauthorized`, `Forbidden`, `NotFound`, `Conflict`, `Internal`).

4. **Compatibilidad con YugabyteDB (YSQL):**
   - Toda nueva tabla debe usar `UUID PRIMARY KEY DEFAULT gen_random_uuid()` e incluir `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`.
   - Las migraciones nuevas se agregan en `migrations/` con prefijo secuencial (`0002_nombre.sql`) y se aplican automáticamente al iniciar la aplicación mediante `sqlx::migrate!("./migrations")`.

5. **No consultar `BYTEA` innecesariamente:**
   - Nunca incluyas la columna `data` de `note_attachments` en listados o metadatos; consúltala únicamente en `get_attachment` cuando se vaya a transmitir el binario de una imagen específica.

6. **Verificación E2E:**
   Con el servidor activo (`cargo run`), ejecuta el script de regresión para validar todos los flujos:
   ```bash
   ./scripts/test_api.sh
   ```

---

## 7. Cómo Habilitar el Login con Google (OAuth 2.0 / Google Identity Services)

El backend ya incluye soporte nativo en `POST /api/auth/google` (`src/auth.rs`) para validar tokens de identidad (`id_token`) emitidos por Google Sign-In. No requiere almacenar un `client_secret` en el backend porque utiliza el flujo de verificación de `id_token` firmado por Google.

### Paso 1: Obtener el `GOOGLE_CLIENT_ID` en Google Cloud Console

1. Entra a [Google Cloud Console](https://console.cloud.google.com/) y crea o selecciona un proyecto.
2. Ve a **APIs y servicios → Pantalla de consentimiento de OAuth** (*OAuth consent screen*):
   - Selecciona tipo **Externo** (*External*).
   - Completa el nombre de la aplicación (ej. `Notiz`), correo de soporte y guarda.
   - Si está en modo *Testing*, agrega tu correo electrónico en la sección **Usuarios de prueba** (*Test users*).
3. Ve a **APIs y servicios → Credenciales** (*Credentials*) → **Crear credenciales → ID de cliente de OAuth** (*OAuth client ID*):
   - **Tipo de aplicación:** `Aplicación web` (*Web application*).
   - **Orígenes de JavaScript autorizados** (*Authorized JavaScript origins*): agrega la URL donde correrá tu frontend (por ejemplo `http://localhost:5173`, `http://localhost:3000` y `http://127.0.0.1:5173`).
4. Copia el **ID de cliente** generado (tiene el formato `1234567890-abcdefg.apps.googleusercontent.com`).

---

### Paso 2: Configurar la variable en `.env`

Abre el archivo `.env` en la raíz de `notiz-back-api` y asigna tu Client ID:

```dotenv
GOOGLE_CLIENT_ID=1234567890-abcdefg.apps.googleusercontent.com
```

Reinicia el backend (`cargo run`) para que tome la nueva variable de entorno.

---

### Paso 3: Cómo conectarlo desde el Frontend

1. En el frontend (React, Vue, Svelte, Astro o HTML/JS puro), utiliza el botón oficial de **Google Identity Services** (o librerías como `@react-oauth/google` con el componente `<GoogleLogin />`).
2. Cuando el usuario inicia sesión en el popup de Google, Google devuelve un objeto de respuesta cuya propiedad `credential` es el **`id_token`** (un JWT firmado por Google).
3. Envía ese `id_token` al backend:

```javascript
const response = await fetch("http://localhost:3000/api/auth/google", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    id_token: googleResponse.credential,
  }),
});

const { token, user } = await response.json();
// Guarda `token` (JWT propio de Notiz) para enviarlo en `Authorization: Bearer ${token}`
```

---

### Paso 4: Qué hace el Backend internamente (`POST /api/auth/google`)

1. Verifica que `GOOGLE_CLIENT_ID` esté configurado en `.env`.
2. Consulta a `https://oauth2.googleapis.com/tokeninfo?id_token=<token>` para validar la firma y expiración del token de Google.
3. Comprueba que el campo `aud` (*audience*) coincida exactamente con tu `GOOGLE_CLIENT_ID` y que `email_verified` sea `"true"`.
4. **Resolución de cuenta en YugabyteDB / PostgreSQL:**
   - Si el usuario **no existe**, crea automáticamente el registro en `users` con `google_sub`, `email`, `password_hash = NULL` y un `username` único generado a partir de su nombre/correo.
   - Si el usuario **ya se había registrado antes con correo y contraseña** usando ese mismo email, vincula automáticamente su `google_sub` a la cuenta existente para que pueda entrar con cualquiera de los dos métodos sin perder sus notas ni carpetas.
5. Devuelve el JWT propio del backend (`AuthResponse`: `{ token, user }`), idéntico al de `/api/auth/login`.


