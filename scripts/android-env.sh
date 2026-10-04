# Source this before building for Android:  . scripts/android-env.sh
# Sets up the JDK, SDK and NDK, and puts the NDK's tools on PATH (SQLCipher builds OpenSSL from source,
# which needs the cross compiler, ar and ranlib; see docs/build-notes.md).
export JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home -v 17)}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export NDK_VERSION="${NDK_VERSION:-30.0.14904198}"
export NDK_HOME="$ANDROID_HOME/ndk/$NDK_VERSION"
export ANDROID_NDK_HOME="$NDK_HOME"
_ndk_bin="$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"
export PATH="$_ndk_bin:$ANDROID_HOME/platform-tools:$PATH"
export RANLIB="$_ndk_bin/llvm-ranlib"
for _arch in aarch64:aarch64 armv7:armv7a x86_64:x86_64 i686:i686; do
  _rust="${_arch%%:*}"; _clang="${_arch##*:}"
  _triple="${_rust}-linux-android"; [ "$_rust" = armv7 ] && _triple="armv7-linux-androideabi"
  _clang_name="${_clang}-linux-android"; [ "$_rust" = armv7 ] && _clang_name="armv7a-linux-androideabi"
  _var="$(echo "$_triple" | tr - _)"
  export "CC_${_var}=$_ndk_bin/${_clang_name}30-clang"
  export "AR_${_var}=$_ndk_bin/llvm-ar"
  export "CARGO_TARGET_$(echo "$_var" | tr a-z A-Z)_LINKER=$_ndk_bin/${_clang_name}30-clang"
done
unset _arch _rust _clang _triple _clang_name _var _ndk_bin
