#!/usr/bin/env bash
# Source before building to bake in the Google Desktop OAuth client:
#   . scripts/google-env.sh && npm run tauri build
# Reads the client JSON downloaded from Google Cloud (kept outside the repo, never committed).
f=${EDGE_GOOGLE_CLIENT_JSON:-$HOME/.config/edge-sync/google-desktop-client.json}
if [ -f "$f" ]; then
  eval "$(python3 -c 'import json,sys,shlex;c=json.load(open(sys.argv[1]))["installed"];print("export EDGE_GOOGLE_CLIENT_ID="+shlex.quote(c["client_id"])+" EDGE_GOOGLE_CLIENT_SECRET="+shlex.quote(c["client_secret"]))' "$f")"
else
  echo "google-env.sh: $f not found; Google sign-in will be disabled in this build" >&2
fi
