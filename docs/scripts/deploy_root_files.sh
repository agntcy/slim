#!/bin/bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# Deploy root-level site files (404.html, robots.txt, llms.txt) to the root of
# the gh-pages branch. mike publishes versioned docs into subdirectories
# (/dir/latest/, /dir/<version>/), so these files must be placed at the branch
# root separately. Designed to be called from CI after `mike deploy --push`.
#
# Arguments:
#   $1: GitHub repository (e.g. "agntcy/dir")
#
# Environment:
#   GH_TOKEN: token for authenticated clone/push. Read from the environment so
#             it never appears on the command line (argv), in the remote URL, or
#             in .git/config.

set -euo pipefail

# Files copied from SOURCE_DIR to the gh-pages root.
FILES_TO_DEPLOY=("404.html" "robots.txt" "llms.txt")
# Source directory (published Markdown/content root) relative to repo root.
SOURCE_DIR="docs/content"

REPO_NAME="${1:-}"
if [ -z "${REPO_NAME}" ]; then
  echo "Error: missing required argument <owner/repo>." >&2
  echo "Usage: GH_TOKEN=<token> $0 <owner/repo>" >&2
  exit 1
fi
: "${GH_TOKEN:?GH_TOKEN environment variable is required}"

echo "Deploying root-level site files for repository: ${REPO_NAME}"
echo "Files to deploy: ${FILES_TO_DEPLOY[*]}"

# Credential helper that reads the token from the environment at runtime.
# The single-quoted string stores the literal "$GH_TOKEN" (the variable name),
# so the secret value is never embedded in argv or persisted to .git/config.
CRED_HELPER='!f() { test "$1" = get && printf "username=x-access-token\npassword=%s\n" "${GH_TOKEN}"; }; f'

# Resolve repo root so the script works regardless of caller CWD.
REPO_ROOT="$(git rev-parse --show-toplevel)"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "${WORKDIR}"' EXIT

git -c credential.helper= -c credential.helper="${CRED_HELPER}" \
  clone --branch=gh-pages --single-branch --depth=1 \
  "https://github.com/${REPO_NAME}.git" \
  "${WORKDIR}/gh-pages-deploy"

cd "${WORKDIR}/gh-pages-deploy"

for file in "${FILES_TO_DEPLOY[@]}"; do
  SOURCE_FILE="${REPO_ROOT}/${SOURCE_DIR}/${file}"
  if [ -f "${SOURCE_FILE}" ]; then
    echo "Copying ${file}..."
    cp "${SOURCE_FILE}" "./${file}"
    git add "${file}"
  else
    echo "Warning: source file not found, skipping: ${SOURCE_FILE}" >&2
  fi
done

if git diff --staged --quiet; then
  echo "Root files are up-to-date. No new commit needed."
else
  echo "Committing and pushing updated root files..."
  git config user.name "github-actions[bot]"
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git commit -m "docs: deploy root-level site files"
  # Reuse the runtime credential helper (clone's -c is not persisted to config).
  git -c credential.helper= -c credential.helper="${CRED_HELPER}" push origin gh-pages
fi

echo "Root file deployment complete."
