# shellcheck shell=bash
# Sourced by prepare.sh and build.sh.

# Sets $token to a fresh GitHub OIDC token whose audience is the Cloud origin $1, masked in the log.
oidc_token() {
    local audience
    audience=$(jq -rn --arg value "$1" '$value | @uri')
    token=$(curl -fsS -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
        "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=$audience" | jq -r '.value')
    echo "::add-mask::$token"
}
