# Wisecrow agent instructions

## Web changes ship in the Android app

The phone app launches `wisecrow_web::app` from `wisecrow-mobile/src/online.rs`. A change under `wisecrow-web` is not done until `wisecrow-mobile/dist/android-arm64/app-debug.apk` is rebuilt from that source.

Rebuild from `wisecrow-mobile` with Rust 1.97.0 from rustup. That toolchain has the `aarch64-linux-android` target. The Devbox rustc does not, so do not let it win `PATH`.

```bash
dx bundle --platform android --features mobile \
  --target aarch64-linux-android --package-types apk \
  --out-dir dist/android-arm64 --locked
```

Use the Android SDK and NDK 27.2.12479018 already installed. Do not download another NDK. After the build, confirm the APK contains the new text and no longer contains the text it replaced. Install only when a device is attached and the signature matches the installed package.
