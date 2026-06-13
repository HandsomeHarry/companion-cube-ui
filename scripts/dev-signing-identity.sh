#!/usr/bin/env bash
# Create a stable self-signed code-signing identity for local development.
#
# Why this exists: ad-hoc signing (codesign -s -) gives the app a designated
# requirement of `cdhash H"..."` — the literal hash of the binary. macOS TCC
# stores Screen Recording / Accessibility grants against that requirement, so
# every rebuild (new binary → new hash) looks like a brand-new app and macOS
# re-prompts. Signing with a real certificate makes the requirement
# `identifier "com.companioncube.daemon" and certificate leaf = H"<cert>"`,
# which is constant across rebuilds — so a grant given once persists.
#
# The cert is self-signed and lives in a throwaway keychain. It protects
# nothing (anyone can regenerate it); it exists only to give TCC a stable
# identity to remember. Idempotent: re-running is a no-op once set up.
set -euo pipefail

CERT_CN="Companion Cube Dev"
KEYCHAIN="$HOME/Library/Keychains/ccube-signing.keychain-db"
KCPASS="ccube-dev"   # local dev keychain only — not your login keychain

# Already set up? Then we're done. (No -v: a self-signed cert is reported
# untrusted by policy, but codesign can still sign with it.)
if security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q "$CERT_CN" \
   && security list-keychains -d user | grep -q "ccube-signing"; then
  echo "Signing identity '$CERT_CN' already present."
  exit 0
fi

echo "==> Creating self-signed code-signing certificate"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -days 3650 \
  -subj "/CN=$CERT_CN" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null

# -legacy + SHA1 MAC: OpenSSL 3's modern PKCS#12 MAC/encryption is unreadable
# by Apple's Security framework, which fails import with "MAC verification
# failed". These flags emit the legacy encoding macOS understands.
openssl pkcs12 -export -out "$TMP/identity.p12" \
  -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -passout "pass:$KCPASS" -name "$CERT_CN" \
  -legacy -macalg sha1 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES 2>/dev/null

echo "==> Creating dedicated signing keychain"
security delete-keychain "$KEYCHAIN" 2>/dev/null || true
security create-keychain -p "$KCPASS" "$KEYCHAIN"
security set-keychain-settings "$KEYCHAIN"          # no auto-lock timeout
security unlock-keychain -p "$KCPASS" "$KEYCHAIN"

echo "==> Importing identity (pre-authorizing codesign so it won't prompt)"
security import "$TMP/identity.p12" -k "$KEYCHAIN" -P "$KCPASS" -T /usr/bin/codesign
# Allow codesign to use the key non-interactively.
security set-key-partition-list -S apple-tool:,apple: -s -k "$KCPASS" "$KEYCHAIN" >/dev/null 2>&1

# codesign searches the default keychain search list for the identity by
# name, so the signing keychain must be on it. Add it once, preserving the
# user's existing keychains and avoiding duplicates.
if ! security list-keychains -d user | grep -q "ccube-signing"; then
  echo "==> Adding signing keychain to the search list"
  EXISTING=$(security list-keychains -d user | sed -E 's/^[[:space:]]*"//; s/"$//')
  # shellcheck disable=SC2086
  security list-keychains -d user -s "$KEYCHAIN" $EXISTING
fi

echo "==> Done. Identity '$CERT_CN' is ready in $KEYCHAIN"
