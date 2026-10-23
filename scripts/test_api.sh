#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"
SUFFIX="$(date +%s)"
ALICE_USER="alice_${SUFFIX}"
ALICE_EMAIL="alice_${SUFFIX}@example.com"
BOB_USER="bob_${SUFFIX}"
BOB_EMAIL="bob_${SUFFIX}@example.com"

echo "=== 1. Health Check ==="
curl -sS "${BASE_URL}/api/health"
echo -e "\n"

echo "=== 2. Registrar Usuario 1 (${ALICE_USER}) ==="
ALICE_RES=$(curl -sS -X POST "${BASE_URL}/api/auth/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\": \"${ALICE_USER}\", \"email\": \"${ALICE_EMAIL}\", \"password\": \"secreto1234\"}")
echo "${ALICE_RES}"
echo -e "\n"

echo "=== 3. Login de Alice (POST /api/auth/login) ==="
LOGIN_RES=$(curl -sS -X POST "${BASE_URL}/api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"identifier\": \"${ALICE_EMAIL}\", \"password\": \"secreto1234\"}")
echo "${LOGIN_RES}"
ALICE_TOKEN=$(echo "${LOGIN_RES}" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
echo -e "\n"

echo "=== 4.Registrar Usuario 2 (${BOB_USER}) ==="
BOB_RES=$(curl -sS -X POST "${BASE_URL}/api/auth/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\": \"${BOB_USER}\", \"email\": \"${BOB_EMAIL}\", \"password\": \"secreto1234\"}")
echo "${BOB_RES}"
BOB_TOKEN=$(echo "${BOB_RES}" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
BOB_ID=$(echo "${BOB_RES}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
echo -e "\n"

echo "=== 5. Crear Carpeta con Alice ==="
FOLDER_RES=$(curl -sS -X POST "${BASE_URL}/api/folders" \
  -H "Authorization: Bearer ${ALICE_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"name": "Proyectos Rust"}')
echo "${FOLDER_RES}"
FOLDER_ID=$(echo "${FOLDER_RES}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
echo -e "\n"

echo "=== 6. Crear Nota Markdown con Tags en la Carpeta ==="
NOTE_RES=$(curl -sS -X POST "${BASE_URL}/api/notes" \
  -H "Authorization: Bearer ${ALICE_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"Arquitectura del Backend\",
    \"body\": \"# Notiz API\n\nNota en **Markdown** con Axum y YugabyteDB.\",
    \"tags\": [\"Rust\", \"Axum\", \"YugabyteDB\"],
    \"folder_id\": \"${FOLDER_ID}\"
  }")
echo "${NOTE_RES}"
NOTE_ID=$(echo "${NOTE_RES}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
echo -e "\n"

echo "=== 7. Subir Imagen Adjunta (BLOB en BD) ==="
TMP_IMG=$(mktemp --suffix=.png)
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\nIDATx\x9cc\x00\x01\x00\x00\x05\x00\x01\r\n-\xb4\x00\x00\x00\x00IEND\xaeB`\x82' > "${TMP_IMG}"

ATTACH_RES=$(curl -sS -X POST "${BASE_URL}/api/notes/${NOTE_ID}/attachments" \
  -H "Authorization: Bearer ${ALICE_TOKEN}" \
  -F "file=@${TMP_IMG};type=image/png;filename=diagrama.png")
rm -f "${TMP_IMG}"
echo "${ATTACH_RES}"
ATTACH_ID=$(echo "${ATTACH_RES}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
echo -e "\n"

echo "=== 8. Verificar que Bob NO puede ver la nota privada de Alice ==="
curl -sS -i "${BASE_URL}/api/notes/${NOTE_ID}" \
  -H "Authorization: Bearer ${BOB_TOKEN}" | head -n 15
echo -e "\n"

echo "=== 9. Alice comparte la nota con Bob (por username o correo) con permiso 'edit' ==="
curl -sS -X POST "${BASE_URL}/api/notes/${NOTE_ID}/shares" \
  -H "Authorization: Bearer ${ALICE_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"recipient\": \"${BOB_USER}\", \"permission\": \"edit\"}"
echo -e "\n"

echo "=== 10. Ahora Bob puede ver y editar la nota compartida y descargar el BLOB ==="
curl -sS -X PUT "${BASE_URL}/api/notes/${NOTE_ID}" \
  -H "Authorization: Bearer ${BOB_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"body": "# Notiz API\n\nEditado colaborativamente por Bob."}'
echo -e "\n"

echo "=== 11. Descargar imagen adjunta (BLOB) con el token de Bob ==="
curl -sS -I "${BASE_URL}/api/notes/${NOTE_ID}/attachments/${ATTACH_ID}" \
  -H "Authorization: Bearer ${BOB_TOKEN}"

echo "=== 12. Revocar compartido, eliminar adjunto, nota y carpeta ==="
curl -sS -X DELETE "${BASE_URL}/api/notes/${NOTE_ID}/shares/${BOB_ID}" -H "Authorization: Bearer ${ALICE_TOKEN}" -w "Unshare HTTP: %{http_code}\n"
curl -sS -X DELETE "${BASE_URL}/api/notes/${NOTE_ID}/attachments/${ATTACH_ID}" -H "Authorization: Bearer ${ALICE_TOKEN}" -w "Delete Attachment HTTP: %{http_code}\n"
curl -sS -X DELETE "${BASE_URL}/api/notes/${NOTE_ID}" -H "Authorization: Bearer ${ALICE_TOKEN}" -w "Delete Note HTTP: %{http_code}\n"
curl -sS -X DELETE "${BASE_URL}/api/folders/${FOLDER_ID}" -H "Authorization: Bearer ${ALICE_TOKEN}" -w "Delete Folder HTTP: %{http_code}\n"

echo -e "\n✅ Todas las pruebas finalizaron correctamente."
