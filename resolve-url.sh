#!/usr/bin/env bash
# Resolve one HTTP(S) URL through redirects and print the final URL.
# Usage: bash resolve-url.sh 'https://example.com/redirect'
# Requires curl. Errors go to stderr and return a nonzero exit status.
# A final HTTP error response still counts as a resolved URL.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    printf 'Usage: %s <http-or-https-url>\n' "$0" >&2
    exit 1
fi

case "$1" in
    [hH][tT][tT][pP]://*|[hH][tT][tT][pP][sS]://*) ;;
    *) printf 'Error: URL must use HTTP or HTTPS.\n' >&2; exit 1 ;;
esac

if ! command -v curl >/dev/null 2>&1; then
    printf 'Error: curl is required but was not found.\n' >&2
    exit 1
fi

# Capture first so a failed request never prints a misleading result to stdout.
resolved_url=$(curl \
    --location \
    --silent \
    --show-error \
    --globoff \
    --output /dev/null \
    --write-out '%{url_effective}' \
    --connect-timeout 10 \
    --max-time 30 \
    --max-redirs 10 \
    --proto '=http,https' \
    --proto-redir '=http,https' \
    --url "$1")

printf '%s\n' "$resolved_url"
