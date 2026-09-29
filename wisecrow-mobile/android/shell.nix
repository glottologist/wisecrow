{
  pkgs ?
    import <nixpkgs> {
      config = {
        allowUnfree = true;
        android_sdk.accept_license = true;
      };
    },
}: let
  android = pkgs.androidenv.composeAndroidPackages {
    platformVersions = ["35" "36"];
    buildToolsVersions = ["34.0.0" "35.0.0"];
    cmdLineToolsVersion = "20.0";
    platformToolsVersion = "37.0.0";
    includeNDK = true;
    ndkVersions = ["27.2.12479018"];
    includeCmake = true;
    cmakeVersions = ["4.1.2"];
    includeEmulator = false;
  };
  sdk = "${android.androidsdk}/libexec/android-sdk";
  ndk = "${sdk}/ndk/27.2.12479018";
in
  pkgs.mkShell {
    packages = [pkgs.rustup pkgs.jdk21 pkgs.gradle pkgs.pkg-config];

    ANDROID_HOME = sdk;
    ANDROID_SDK_ROOT = sdk;
    ANDROID_NDK_HOME = ndk;
    NDK_HOME = ndk;
    JAVA_HOME = pkgs.jdk21.home;
    RUSTUP_TOOLCHAIN = "1.97.0";
    HOST_CC = "${pkgs.stdenv.cc}/bin/cc";
    HOST_CXX = "${pkgs.stdenv.cc}/bin/c++";
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
    GRADLE_USER_HOME = builtins.toString ../../target/android-gradle;
    GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/35.0.0/aapt2";

    shellHook = ''
      export PATH="${pkgs.rustup}/bin:${sdk}/platform-tools:$PATH"
      mkdir -p "$GRADLE_USER_HOME/init.d"
      ln -sfn ${./gradle.init.gradle} "$GRADLE_USER_HOME/init.d/wisecrow.init.gradle"
    '';
  }
