#!/usr/bin/env bash
# Add the NDK toolchain only while configuring Android; --build/--version pass through.
set -euo pipefail
android=false
abi=''
for arg in "$@"; do
  case "$arg" in
    -DCMAKE_SYSTEM_NAME=Android) android=true ;;
    -DCMAKE_SYSTEM_PROCESSOR=aarch64) abi=arm64-v8a ;;
    -DCMAKE_SYSTEM_PROCESSOR=armv7-a) abi=armeabi-v7a ;;
    -DCMAKE_SYSTEM_PROCESSOR=x86_64) abi=x86_64 ;;
    -DCMAKE_SYSTEM_PROCESSOR=i686) abi=x86 ;;
  esac
done
if $android; then
  ndk="${NDK_HOME:-${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}}"
  if [[ -z "$ndk" || -z "$abi" ]]; then
    echo 'Android voice needs NDK_HOME and a recognized Android architecture.' >&2
    exit 1
  fi
  scripts="$(cd "$(dirname "$0")" && pwd)"
  exec cmake "$@" "-DCMAKE_TOOLCHAIN_FILE=$ndk/build/cmake/android.toolchain.cmake" \
    "-DANDROID_ABI=$abi" -DANDROID_PLATFORM=android-24 -DANDROID_STL=c++_static \
    "-DCMAKE_PROJECT_INCLUDE=$scripts/android-voice.cmake"
fi
exec cmake "$@"
