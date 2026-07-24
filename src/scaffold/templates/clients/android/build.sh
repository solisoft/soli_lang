#!/bin/bash
# Build without Gradle — aapt2 -> javac -> d8 -> zipalign -> apksigner.
set -euo pipefail
cd "$(dirname "$0")"

: "${ANDROID_HOME:?set ANDROID_HOME to your SDK root}"
BUILD_TOOLS="$ANDROID_HOME/build-tools/35.0.0"
PLATFORM="$ANDROID_HOME/platforms/android-34/android.jar"
OUT=build
APK=app.apk

rm -rf "$OUT"; mkdir -p "$OUT/compiled" "$OUT/gen" "$OUT/classes"

echo "==> Compiling resources"
"$BUILD_TOOLS/aapt2" compile --dir res -o "$OUT/compiled/res.zip"

echo "==> Linking resources"
"$BUILD_TOOLS/aapt2" link \
  -o "$OUT/base.apk" \
  -I "$PLATFORM" \
  --manifest AndroidManifest.xml \
  --java "$OUT/gen" \
  --min-sdk-version 24 \
  --target-sdk-version 34 \
  "$OUT/compiled/res.zip"

echo "==> Compiling Java"
javac -source 17 -target 17 -nowarn \
  -classpath "$PLATFORM" \
  -d "$OUT/classes" \
  $(find src "$OUT/gen" -name '*.java')

echo "==> Dexing"
jar cf "$OUT/classes.jar" -C "$OUT/classes" .
"$BUILD_TOOLS/d8" --lib "$PLATFORM" --output "$OUT" "$OUT/classes.jar"

echo "==> Packaging"
cp "$OUT/base.apk" "$OUT/unsigned.apk"
cd "$OUT" && zip -q -u unsigned.apk classes.dex && cd ..

echo "==> Align + sign"
"$BUILD_TOOLS/zipalign" -f 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
if [ ! -f debug.keystore ]; then
  keytool -genkeypair -v -keystore debug.keystore -storepass android \
    -alias androiddebugkey -keypass android -keyalg RSA -keysize 2048 \
    -validity 10000 -dname "CN=Debug"
fi
"$BUILD_TOOLS/apksigner" sign --ks debug.keystore --ks-pass pass:android \
  --key-pass pass:android --out "$APK" "$OUT/aligned.apk"
echo "Built $APK"
