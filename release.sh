#!/bin/zsh
# Baut nrzCapture: App mit fester Signatur-Regel (Bildschirmaufnahme-Freigabe bleibt über Updates erhalten),
# DMG, signiertes Update-Paket und latest.json.  Aufruf: ./release.sh
set -e
cd "${0:A:h}"
U=/Users/nicorenezweers
export RUSTUP_HOME=$U/.rustup CARGO_HOME=$U/.cargo PATH=$U/.cargo/bin:$PATH
export TAURI_SIGNING_PRIVATE_KEY="$(cat $U/.tauri/nrzcapture.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
V=$(sed -n 's/^version = "\(.*\)"/\1/p' src-tauri/Cargo.toml)
OUT=target/release/bundle; APP=$OUT/macos/nrzCapture.app; TGZ=$OUT/macos/nrzCapture.app.tar.gz

npx --yes @tauri-apps/cli build --bundles app
# Ad-hoc-Signatur, aber Regel nur auf die Bundle-ID statt auf den Hash jedes Builds.
codesign --force --deep -s - -r='designated => identifier "com.nrz.capture"' "$APP"
codesign -v "$APP"

rm -f $TGZ $TGZ.sig && tar -czf $TGZ -C $OUT/macos nrzCapture.app
npx --yes @tauri-apps/cli signer sign $TGZ >/dev/null

mkdir -p $OUT/dmg && DMG=$OUT/dmg/nrzCapture_${V}_aarch64.dmg && rm -f $DMG
S=$(mktemp -d) && cp -R "$APP" $S/ && ln -s /Applications $S/Programme
hdiutil create -quiet -volname nrzCapture -srcfolder $S -format UDZO $DMG && rm -rf $S

cat > latest.json <<EOF
{"version":"$V","pub_date":"$(date -u +%Y-%m-%dT%H:%M:%SZ)","platforms":{"darwin-aarch64":{"signature":"$(cat $TGZ.sig)","url":"http://100.78.107.162:8390/nicorenezweers/nrzCapture/releases/download/v$V/nrzCapture.app.tar.gz"}}}
EOF
echo "Fertig: $DMG"
