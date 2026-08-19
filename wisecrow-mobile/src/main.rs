#[cfg(target_os = "android")]
fn main() {
    use std::sync::Arc;

    use wisecrow_mobile::platform::AndroidPlatform;

    tracing::info!("Starting Wisecrow mobile");
    let platform = match AndroidPlatform::new() {
        Ok(platform) => Arc::new(platform),
        Err(_) => {
            tracing::error!("Android platform initialization failed");
            return;
        }
    };
    dioxus::LaunchBuilder::mobile()
        .with_context(platform)
        .launch(wisecrow_mobile::app);
}

#[cfg(all(not(target_os = "android"), feature = "desktop"))]
fn main() {
    use std::sync::Arc;

    use wisecrow_mobile::platform::DesktopPlatform;

    tracing::info!("Starting Wisecrow mobile");
    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(_) => {
            tracing::error!("Desktop platform root is unavailable");
            return;
        }
    };
    let platform = match DesktopPlatform::new(&root) {
        Ok(platform) => Arc::new(platform),
        Err(_) => {
            tracing::error!("Desktop platform initialization failed");
            return;
        }
    };
    dioxus::LaunchBuilder::desktop()
        .with_context(platform)
        .launch(wisecrow_mobile::app);
}

#[cfg(all(not(target_os = "android"), not(feature = "desktop")))]
fn main() {
    tracing::info!("Starting Wisecrow mobile");
    dioxus::prelude::launch(wisecrow_mobile::app);
}
