#!/bin/sh
set -e

GITHUB_TOKEN=$(cat /run/secrets/github_token)
TAG="${DRONE_TAG}"
REPO="${DRONE_REPO}"

echo "Creating release: OpenFootball Release ${TAG}"

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  -H "Authorization: token ${GITHUB_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"tag_name\":\"${TAG}\",\"name\":\"OpenFootball ${TAG}\"}" \
  "https://api.github.com/repos/${REPO}/releases")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" -ge 400 ]; then
  echo "Failed to create release (HTTP ${HTTP_CODE}):"
  echo "$BODY"
  exit 1
fi

RELEASE_ID=$(echo "$BODY" | jq -r '.id')
echo "Release created: id=${RELEASE_ID}"

for FILE in /release/*; do
  NAME=$(basename "$FILE")
  echo "Uploading ${NAME}..."

  UPLOAD_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
    -H "Authorization: token ${GITHUB_TOKEN}" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@${FILE}" \
    "https://uploads.github.com/repos/${REPO}/releases/${RELEASE_ID}/assets?name=${NAME}")

  UPLOAD_CODE=$(echo "$UPLOAD_RESPONSE" | tail -1)
  if [ "$UPLOAD_CODE" -ge 400 ]; then
    echo "Failed to upload ${NAME} (HTTP ${UPLOAD_CODE}):"
    echo "$UPLOAD_RESPONSE" | sed '$d'
    exit 1
  fi

  echo "Uploaded ${NAME}"
done

echo "Release published successfully"
