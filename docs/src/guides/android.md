# Android: build, install and use

Open `https://wisecrow.glottologist.co.uk:8443` in your phone's browser to use
Wise Crow today. The native Android application still needs the connections
described below before it can sign in, download lessons or work offline.

This guide covers a Linux workstation and a USB-connected Android phone. Build
on the workstation where the repository lives; calypso continues to run the
server. The Android build does not require a local PostgreSQL database.

## Current readiness

**Checked on 28 September 2026: no working APK installation has been verified.**
These are application gaps, so installing more SDK packages will not resolve
them:

| Area | Current state |
| --- | --- |
| Launchable package | `wisecrow-mobile/Dioxus.toml` selects `android/AndroidManifest.xml` as the complete application manifest. It has permissions and an `<application>` element, but no activity or launcher intent. Dioxus CLI 0.7.10 copies this file instead of its generated launcher manifest. This needs correction before the APK can be opened normally. |
| Server connection and local data | `src/transport/api.rs` holds a server origin but does not implement `MobileApi`. The Android entry point in `src/main.rs` launches `app`, without the store/API/media contexts supplied by `app_with_media` in `src/lib.rs`. |
| User interface | `src/components/home.rs` displays a fixed “Local storage is ready.” message. That message does not check the database. Learn and N-back are placeholders; the navigation has only Home. There is no login, server-settings or download screen. Grammar components exist but need the missing store context and downloaded data. |

The `src/` paths in this table are relative to `wisecrow-mobile/`. The underlying
storage, sync and grammar code has automated tests, but that is not evidence of
a working installed app. Fast mode is not a route in the mobile shell.

**The following commands are the development build/install procedure. They do
not complete these missing features.** Fix the launcher manifest first; finish
the application connections before expecting a usable offline learning app.

## Prepare the workstation

Allow approximately **30–60 minutes** for a first-time SDK/toolchain setup,
depending on downloads. The examples use Bash.

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
   | SDK Tools | Android SDK Platform-Tools, Command-line Tools, Build-Tools 35.0.0 |
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
   an Android build. On NixOS, use Android tools supplied by your Nix environment
   if downloaded tools cannot execute; this repository does not provide a
   complete Android Nix shell.

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
   when release signing has not been configured.

2. Check that the command succeeded and locate the output:

   ```bash
   rg --files --hidden --no-ignore dist/android-arm64 -g '*.apk'
   ```

   Use the APK path printed by this command. Do not assume an APK left by a
   previous failed build contains your latest changes.

3. Inspect its signature and packaged manifest, substituting that path:

   ```bash
   export WISECROW_APK="/absolute/path/to/the-built.apk"
   "$ANDROID_HOME/build-tools/35.0.0/apksigner" verify "$WISECROW_APK"
   "$ANDROID_HOME/build-tools/35.0.0/aapt" dump badging "$WISECROW_APK"
   ```

   Check for package `org.wisecrow.mobile`, the intended SDK requirements and a
   `launchable-activity` entry. **Stop here if the launcher entry is absent**;
   the current custom manifest needs fixing. Signature verification is
   described in the official [apksigner reference](https://developer.android.com/tools/apksigner).

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

4. Open Wise Crow from the phone's app launcher. Alternatively, resolve its
   launch activity for diagnosis:

   ```bash
   adb -s "$WISECROW_DEVICE" shell cmd package resolve-activity --brief org.wisecrow.mobile
   ```

   An installed package with no launch activity is a packaging problem, not a
   server connection problem.

## Use on Android

For the current usable interface:

1. Open `https://wisecrow.glottologist.co.uk:8443` in the phone's browser.
2. Sign in with your existing Wise Crow account.
3. Select the language and learning mode on the website.

The website needs network access. Installing the native shell does not add
offline support to the website.

For the **native APK**, after the launcher is fixed, the current code aims to
show Home and its fixed storage message. There are no working sign-in or
download instructions to follow yet. Rebuilding the server or copying grammar
PDFs to the phone will not fill those missing application connections.

The native release should be accepted only after these flows are implemented
and verified on a device:

| Flow | What must work |
| --- | --- |
| Connect | Configure `https://wisecrow.glottologist.co.uk:8443` as the server origin, authenticate and register the device. The URL is the server root, not a `/fast` or `/api` route. |
| Download | Select a language, complete the initial vocabulary and grammar sync, and display completion. |
| Learn offline | Open downloaded lessons without a network connection and save answers locally. |
| Reconnect | Upload pending answers and refresh progress without duplicates. |
| Audio | Download an example clip while online and play the cached clip in airplane mode. |

These are acceptance requirements, not existing menu labels. The current router
and screens do not expose this workflow. The
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
| `INSTALL_FAILED_NO_MATCHING_ABIS` | Compare `adb -s "$WISECROW_DEVICE" shell getprop ro.product.cpu.abilist` with the Rust build target. |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` | Rebuild with the signing key used for the installed app. |
| Install succeeds but no app icon appears | Inspect the packaged manifest's launcher activity; the current override omits it. |
| Only Home appears, with no login or download controls | This matches the current shell. The missing application UI and wiring need implementation. |

For a startup failure, collect the crash buffer after reproducing it:

```bash
adb -s "$WISECROW_DEVICE" logcat -b crash -d
```

Build diagnostics can be collected by repeating the build with `--verbose`.
Review logs for credentials or private content before sharing them.

## What was checked for this guide

The commands were checked against the installed `dx` 0.7.10 help and Android
packaging source, the repository configuration and entry point, and official
Android/Dioxus documentation. `dx doctor --offline` found the Rust Android
targets here, but no configured SDK, NDK or Java home. No APK build, installation,
release signing or phone playback was verified.

Start with `dx doctor --offline` from `wisecrow-mobile/` to see the workstation's
remaining setup requirements.
