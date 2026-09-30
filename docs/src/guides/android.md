# Android: build, install and use

Open **WisecrowMobile** and sign in with your existing Wise Crow account.
The Android app now bundles the same Dioxus interface as the website and
connects to `https://wisecrow.glottologist.co.uk:8443`. Lessons require a network
connection; offline downloads are not yet exposed by the app.

This guide covers a Linux workstation and a USB-connected Android phone. Build
on the workstation where the repository lives; calypso continues to run the
server. The Android build does not require a local PostgreSQL database.

## Current readiness

**Checked on 30 September 2026: the signed ARM64 development APK installs and
starts on a Samsung SM-F946B running Android 16. Android reports
`pageSizeCompat=0`.** The shared interface and native adapters compile and pass
automated checks. The phone was locked during inspection, so successful device
sign-in, playback and PDF selection still need confirmation.

| Area | Current state |
| --- | --- |
| Launchable package | The verified APK contains `org.wisecrow.mobile`, Dioxus's `dev.dioxus.main.MainActivity`, native library metadata and the launcher intent. It supports ARM64, declares minimum API 26 and targets API 35. Its launcher label is `WisecrowMobile`. |
| Server connection | `src/online.rs` configures an HTTPS RPC client for the deployed server. Native login uses `/api/mobile/login`, restores identity with `/api/mobile/me`, and stores the bearer token using Android Keystore-backed credentials. Redirects are disabled and TLS verification remains enabled. |
| User interface | The entry point now renders `wisecrow_web::app`, including its sign-in, language selection, Learn, Fast, N-back, PDF Quiz, grammar placement, practice, brainmap, branding and themes. Fast playback and theme changes use the native WebView bridge. PDF selection uses the Android document picker. |
| Offline storage | The existing SQLite/sync implementation and separate offline components remain available as library code. The online entry point does not connect them to the shared UI. There is no download or offline menu. |

The `src/` paths in this table are relative to `wisecrow-mobile/`. Sharing the
web components keeps the online experience aligned across platforms. It does
not supply offline learning or verify every workflow on the phone.

## Prepare the workstation

Allow approximately **30–60 minutes** for a first-time SDK/toolchain setup,
depending on downloads. The examples use Bash.

### NixOS

The repository now provides an Android shell. From the repository root:

```bash
cd wisecrow-mobile
nix-shell android/shell.nix
rustup target add --toolchain 1.97.0 aarch64-linux-android
dx doctor --offline
```

The shell provisions the SDK, Build Tools 34/35, NDK 27.2.12479018, Java and
Gradle, selects Rustup's Rust 1.97.0, and sets the Android environment variables.
The Nix expression accepts the Android SDK licence. It selects Nix's patched
`aapt2` and sets separate host C/C++ compilers so Linux build dependencies do
not accidentally use the Android compiler.

Gradle configuration and caches live in `target/android-gradle/`. The shell
installs `android/gradle.init.gradle` there to select Kotlin 2.2.21 and Android
Gradle Plugin 8.10.1 for Dioxus's generated project. Dioxus CLI 0.7.10 otherwise
selects Kotlin 2.0.20, which cannot compile against OkHttp 5.3.0's Kotlin 2.2
metadata. The platform module declares the same plugin versions. See the
[Gradle init-script documentation](https://docs.gradle.org/current/userguide/init_scripts.html)
and [Android Kotlin compatibility reference](https://developer.android.com/build/kotlin-support).
The Dioxus CLI must already be on `PATH`; its installation is described below.
Stay in this shell for the build commands.

The workspace's `.cargo/config.toml` adds `max-page-size=16384` and
`common-page-size=16384` linker options for ARM64 and x86-64 Android targets.
Dioxus reads these Cargo target settings too. NDK 27 needs these explicit
options for 16 KB ELF and RELRO alignment; host builds are unaffected. See
Android's [16 KB page-size instructions](https://developer.android.com/guide/practices/page-sizes).

### Other Linux setups

1. Open the mobile crate from your existing checkout:

   ```bash
   cd /home/glottologist/development/glottologist/wisecrow/wisecrow-mobile
   ```

   On another machine, replace the path with your checkout location. The
   repository's Devbox configuration supplies Rust development tools, but does
   not provision the Android SDK or NDK.

2. Check Rust and the Dioxus CLI:

   ```bash
   rustc --version
   cargo --version
   dx --version
   rustup target add aarch64-linux-android
   ```

   This guide's command options were checked against **Dioxus CLI 0.7.10**; the
   app pins the top-level Dioxus and Manganis crates to **0.7.9**. If `dx` is
   missing, install this CLI version:

   ```bash
   cargo install dioxus-cli --version 0.7.10 --locked
   ```

   CLI source compilation can take another **10–20 minutes**. Do not change the
   application's dependency versions just to remove a version warning.

3. Install Android Studio and use **Tools → SDK Manager** to install the
   components below. The declared application settings use API 35 and a minimum
   SDK of 26; minimum-device compatibility still needs an actual APK test.

   | SDK Manager tab | Components |
   | --- | --- |
   | SDK Platforms | Android 15 / API 35 |
   | SDK Tools | Android SDK Platform-Tools, Command-line Tools, Build-Tools 34.0.0 and 35.0.0 |
   | SDK Tools | NDK (Side by side), CMake |

   Use a JDK capable of running the Android Gradle build; the checked-in Kotlin
   module targets Java 17. Android Studio supplies a JDK. Record the SDK and NDK
   paths shown by SDK Manager. See the official
   [NDK installation instructions](https://developer.android.com/studio/projects/install-ndk)
   for selecting a specific version.

4. Set the paths in the same terminal. **Replace both angle-bracket values**
   with the actual installed paths/version before running this block:

   ```bash
   export JAVA_HOME="<absolute-path-to-your-jdk>"
   export ANDROID_HOME="$HOME/Android/Sdk"
   export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/<installed-version>"
   export NDK_HOME="$ANDROID_NDK_HOME"
   export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/emulator:$PATH"
   sdkmanager --licenses
   dx doctor --offline
   ```

   Adjust `ANDROID_HOME` if SDK Manager shows a different directory. Read and
   accept the SDK licences when prompted. `dx doctor` should now identify the
   Android SDK, NDK, Java and adb. Its unrelated iOS/macOS warnings do not block
   an Android build. On NixOS, use the repository's Android shell above so
   downloaded tools can execute with the correct host libraries.

5. Apply the repository's Gradle compatibility settings. From
   `wisecrow-mobile/`, keep these variables set for the APK build:

   ```bash
   export GRADLE_USER_HOME="$PWD/../target/android-gradle"
   mkdir -p "$GRADLE_USER_HOME/init.d"
   ln -sfn "$PWD/android/gradle.init.gradle" "$GRADLE_USER_HOME/init.d/wisecrow.init.gradle"
   ```

   The Nix shell already performs this setup. This keeps the override separate
   from your normal Gradle home; use it for this project's Android builds.

The upstream [Dioxus Android setup guide](https://dioxuslabs.com/learn/0.7/guides/platforms/mobile/)
provides the corresponding macOS and Windows environment setup.

## Build an APK for a phone

Allow approximately **10–30 minutes** for the first build after setup. These
commands target a 64-bit ARM phone. Keep the terminal open for installation.

1. From `wisecrow-mobile/`, request an APK explicitly:

   ```bash
   dx bundle --platform android --features mobile \
     --target aarch64-linux-android --package-types apk \
     --out-dir dist/android-arm64 --locked
   ```

   `--package-types apk` matters: CLI 0.7.10 otherwise defaults to an Android App
   Bundle (`.aab`), which is not directly installed with `adb install`. This is
   a development build. The CLI's generated Gradle project uses debug signing
   when release signing has not been configured. The mobile crate depends
   directly on `dioxus-fullstack` for RPC client support. Do not add the
   `fullstack` feature to its direct `dioxus` dependency: CLI 0.7.10 interprets
   that as a request to bundle a host server, even with `--fullstack false`.
   The deployed server continues to run independently.

2. Check that the command succeeded and locate the output:

   ```bash
   rg --files --hidden --no-ignore dist/android-arm64 -g '*.apk'
   ```

   Use the APK path printed by this command. Do not assume an APK left by a
   previous failed build contains your latest changes.

   The verified development build is `dist/android-arm64/app-debug.apk`
   (approximately 99 MiB).

3. Inspect its signature and packaged manifest, substituting that path:

   ```bash
   export WISECROW_APK="/absolute/path/to/the-built.apk"
   "$ANDROID_HOME/build-tools/35.0.0/apksigner" verify "$WISECROW_APK"
   "$ANDROID_HOME/build-tools/35.0.0/aapt" dump badging "$WISECROW_APK"
   ```

   Check for package `org.wisecrow.mobile`, the intended SDK requirements and a
   `launchable-activity` entry. **Stop here if the launcher entry is absent**.
   Signature verification is
   described in the official [apksigner reference](https://developer.android.com/tools/apksigner).

4. Verify native-library alignment in the actual APK:

   ```bash
   export WISECROW_APK_CHECK="$(mktemp -d)"
   unzip "$WISECROW_APK" 'lib/*/*.so' -d "$WISECROW_APK_CHECK"
   "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf" \
     -lW "$WISECROW_APK_CHECK/lib/arm64-v8a/libmain.so"
   "$ANDROID_HOME/build-tools/35.0.0/zipalign" -c -P 16 -v 4 "$WISECROW_APK"
   ```

   Every `LOAD` segment must have alignment at least `0x4000` (16 KB).
   For `GNU_RELRO`, the sum of `VirtAddr` and `MemSiz` must be divisible by
   `0x4000`. `zipalign` must report `Verification successful`. The current
   ARM64 APK contains only `libmain.so`; inspect every `.so` if dependencies
   add more. For an x86-64 APK, use its `lib/x86_64/` path instead.

   A device's API version does not establish its page size. Check with
   `adb -s "$WISECROW_DEVICE" shell getconf PAGE_SIZE` after selecting the
   device below. Runtime testing on a 16 KB system requires a result of `16384`.

For an x86-64 emulator, install the `x86_64-linux-android` Rust target and replace
the build target and output directory accordingly. Use Android Studio's Device
Manager to start the emulator. For development with a running target, the CLI
also accepts `dx serve --platform android --features mobile`.

## Install over USB

Allow **2–5 minutes** once a launchable APK exists.

1. On the phone, enable **Developer options → USB debugging**, connect a data
   cable and accept the computer's debugging authorization prompt. The official
   [hardware-device setup guide](https://developer.android.com/studio/run/device)
   covers phone-specific setup and Linux USB permissions.

2. Identify the phone from your workstation:

   ```bash
   adb devices -l
   ```

   Its status must be `device`. If it says `unauthorized`, unlock the phone and
   accept the prompt. Copy its serial number into the next command; selecting
   it explicitly avoids installing on the wrong device when an emulator is open.

3. Install the APK:

   ```bash
   export WISECROW_DEVICE="<serial-from-adb-devices>"
   adb -s "$WISECROW_DEVICE" install -r "$WISECROW_APK"
   ```

   Expect `Success`. The `-r` option reinstalls the same application while
   retaining its data; see the [adb installation reference](https://developer.android.com/tools/adb#move).
   Matching package signatures are required for an update.

4. Open **WisecrowMobile** from the phone's app launcher. Alternatively, resolve its
   launch activity for diagnosis:

   ```bash
   adb -s "$WISECROW_DEVICE" shell cmd package resolve-activity --brief org.wisecrow.mobile
   ```

   An installed package with no launch activity is a packaging problem, not a
   server connection problem.

### Install by transferring the APK

1. Copy `wisecrow-mobile/dist/android-arm64/app-debug.apk` from the workstation
   to the phone's Downloads folder using USB file transfer or your usual file
   transfer tool.
2. Open the APK in the phone's Files app. If Android requests it, allow that
   app to install this package, then select **Install**.
3. Open **WisecrowMobile** and sign in with your existing account.

## Use on Android

1. Open **WisecrowMobile** while connected to the internet.
2. Sign in with your existing Wise Crow account.
3. Choose your languages under **Start a session**, then select the learning
   mode. **Quiz** opens PDF quizzes; **Choose PDF** opens Android's document picker.

The app restores a saved login when reopened. **Logout** revokes the server
session when reachable and deletes the local credential. Light and dark themes
use the same controls as the website. The website remains available in the
phone's browser at `https://wisecrow.glottologist.co.uk:8443`.

Both interfaces currently require network access. The following offline flows
remain future acceptance requirements:

| Flow | What must work |
| --- | --- |
| Connect offline services | Register the device and connect the profile-scoped local store and sync API to the shared UI. The online login does not initialize an offline profile. |
| Download | Select a language, complete the initial vocabulary and grammar sync, and display completion. |
| Learn offline | Open downloaded lessons without a network connection and save answers locally. |
| Reconnect | Upload pending answers and refresh progress without duplicates. |
| Audio | Download an example clip while online and play the cached clip in airplane mode. |

These are acceptance requirements, not existing menu labels. The
[mobile API reference](../api/wisecrow-mobile.md) describes the underlying pieces.

## Update an installed build

1. Re-run the APK build with the updated source and the same signing key.
2. Verify the newly produced APK and run the same `adb ... install -r` command.

Keep the signing key used for your installations. An APK signed with a different
key cannot replace the installed package. Do not use uninstall/reinstall as the
default fix: uninstalling deletes app-local data, which could include unsynced
answers once offline learning is connected. A distribution release needs its
own maintained signing configuration; `--release` alone does not establish one.

## Troubleshooting

| Symptom | Next action |
| --- | --- |
| `dx doctor` cannot find SDK/NDK/Java | Set the paths in the build terminal and run it again. An `adb` executable alone is not the SDK/NDK. |
| NDK linker missing or cannot execute on NixOS | Check `ANDROID_NDK_HOME` and the executable's host compatibility. Use the Nix-provided Android toolchain/environment. |
| Gradle reports a dependency needs a newer `compileSdk` | Follow the concrete build error and update the app and Kotlin module settings together; installed API 35 alone cannot satisfy a dependency requiring a newer API. |
| Kotlin reports metadata `2.2.0`, expected `2.0.0` | Enter the Nix shell or apply `android/gradle.init.gradle` using the project Gradle home above. Editing the generated Gradle project is not persistent. |
| `INSTALL_FAILED_NO_MATCHING_ABIS` | Compare `adb -s "$WISECROW_DEVICE" shell getprop ro.product.cpu.abilist` with the Rust build target. |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` | Rebuild with the signing key used for the installed app. |
| “This app isn't 16 KB-compatible” / “ELF alignment check failed” | Rebuild with the workspace's Android linker flags and inspect the packaged ELF and ZIP alignment as above. |
| Install succeeds but no app icon appears | Inspect the packaged manifest's launcher activity; rebuild with the corrected manifest override. |
| “Local storage is ready” is still the only screen | This is the older placeholder APK. Rebuild and install the shared-interface version using the same signing key. |
| Login cannot reach the server | Check that the phone can open the deployed HTTPS origin. The native RPC client verifies public TLS certificates and does not use the offline platform's imported-CA settings. |
| No download controls | Offline downloads are not yet connected to the shared interface. Online lessons need a network connection. |

For a startup failure, collect the crash buffer after reproducing it:

```bash
adb -s "$WISECROW_DEVICE" logcat -b crash -d
```

Build diagnostics can be collected by repeating the build with `--verbose`.
Review logs for credentials or private content before sharing them.

## What was checked for this guide

The documented `dx bundle` command completed with Dioxus CLI 0.7.10 and
Rustup 1.97.0. The Nix shell supplied the SDK, NDK and Java; Gradle 9.1.0 used
the repository's Kotlin 2.2.21 / Android Gradle Plugin 8.10.1 overrides.

`apksigner verify --verbose --print-certs` verified the APK's v2 signature
using the Android Debug certificate. `aapt dump badging` confirmed package
`org.wisecrow.mobile`, version `0.1.0`, ARM64, minimum API 26, target API 35
and launcher `dev.dioxus.main.MainActivity`. The packaged manifest references
`wisecrow_network_security_config`; its XML forbids cleartext and retains the
intended system/user certificate trust anchors. The policy lives in the
platform module because Dioxus generates its own `network_security_config.xml`.

On 30 September, all 743 enabled workspace tests passed (191 skipped), as did
all 8 Kotlin unit tests. Workspace and ARM64 Android Clippy passed with warnings
denied. Documentation tests passed with no cases discovered. The native session
tests cover credential persistence, restore, expiration, failed secure storage,
and local logout when remote revocation fails.

The web server feature passed 83 tests (11 skipped), including the signed-out
native route guard. The browser/WASM configuration also passed `cargo check`.
All five Android platform instrumentation tests passed on the connected phone.
Those tests initially exposed an existing credential encryption defect: the
key required randomized encryption but the code supplied its own IV. Encryption
now lets the Keystore provider generate the IV, retaining the authenticated
storage format and strict encryption policy; see
[Android's key generation requirements](https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder.html).
The tests cover encryption, tamper rejection, profile separation, deletion,
app-private paths and bounded PDF validation. They do not exercise interactive
document selection or authenticated learning screens.

The initial APK's four `libmain.so` LOAD segments had `0x1000` alignment,
causing Android's 16 KB compatibility warning. After adding the Cargo linker
settings, the rebuilt APK's only native library has `0x4000` LOAD alignment
and a 16 KB-aligned RELRO end. Its extracted payload matches the inspected
library, and `zipalign -c -P 16 4` and signature verification pass. Reinstalling
changed Android's reported `pageSizeCompat` from `4` to `0`. The phone reports
`PAGE_SIZE=4096`, so runtime on a 16 KB kernel remains untested. The x86-64
linker configuration has not been verified with an emulator build.

The first device launch on 30 September crashed because the generated Manganis
constructor looked for `WisecrowPlatform` using the system class loader from a
native thread. `AndroidPlatform::new` now loads the class through the Activity's
class loader, retains the instance as a global JNI reference and clears handled
Java exceptions before detaching. This follows Android's
[JNI class-loader guidance](https://developer.android.com/ndk/guides/jni-tips#faq:-why-didnt-findclass-find-my-class).

The shared-interface APK passed signature and ZIP alignment inspection and
installed with `adb install -r`. Its SHA-256 is
`59cdefa8cefdc681642c4f93f8a1a068e46ece43da1f8054116d04c0d74d1947`.
Its process remained alive after launch, with no new crash entry. The phone was
locked during inspection; a temporary WebView debug connection timed out and
was removed. Earlier user confirmation of the placeholder Home screen applies
to the previous APK, not this interface. Device sign-in, lessons, playback and
PDF picking remain unverified until the phone is unlocked and used.

Android document selection now registers on the main thread through
`ActivityResultRegistry`, since Rust can request selection after the Activity
has started. The launcher is released on result or Activity destruction, as
required by the [Android registry API](https://developer.android.com/reference/androidx/activity/result/ActivityResultRegistry).
The standalone platform tests run with `gradle -p android/platform
testDebugUnitTest --no-daemon` inside the Nix shell.

This APK uses development signing; it is not a verified production release.
