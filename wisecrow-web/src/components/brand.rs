// Rust guideline compliant 2026-09-21
//! Wise Crow brand assets and the components that place them.
//!
//! Lockups are shipped as two images per placement, one in ink for the light
//! theme and one in cream for the dark theme; the stylesheet shows whichever
//! matches the active `data-theme`. Browser icons and the web manifest keep
//! their file names (no content hash) because the manifest refers to the icon
//! files by path and browsers request the favicon before any script runs.
use dioxus::prelude::*;

const HORIZONTAL_INK: Asset = asset!("/assets/brand/wise-crow-horizontal-ink.svg");
const HORIZONTAL_CREAM: Asset = asset!("/assets/brand/wise-crow-horizontal-cream.svg");
const STACKED_INK: Asset = asset!("/assets/brand/wise-crow-stacked-ink.svg");
const STACKED_CREAM: Asset = asset!("/assets/brand/wise-crow-stacked-cream.svg");
/// Coastal crow illustration shown beside the sign-in card. The bundler
/// re-encodes every raster it can decode (a WebP came out nine times larger),
/// but it cannot decode AVIF and copies it as-is, so AVIF is the primary
/// source and the JPEG is the fallback for browsers without AVIF support.
pub const LANDSCAPE_AVIF: Asset = asset!("/assets/brand/crow-landscape-960.avif");
pub const LANDSCAPE_JPG: Asset = asset!("/assets/brand/crow-landscape-960.jpg");

const FAVICON_SVG: Asset = asset!(
    "/assets/brand/favicon.svg",
    AssetOptions::builder().with_hash_suffix(false)
);
const FAVICON_ICO: Asset = asset!(
    "/assets/brand/favicon.ico",
    AssetOptions::builder().with_hash_suffix(false)
);
const APPLE_TOUCH_ICON: Asset = asset!(
    "/assets/brand/apple-touch-icon.png",
    AssetOptions::builder().with_hash_suffix(false)
);
const WEB_MANIFEST: Asset = asset!(
    "/assets/brand/site.webmanifest",
    AssetOptions::builder().with_hash_suffix(false)
);

// Referenced only from `site.webmanifest` and `style.css`, so nothing in Rust
// reads them; `#[used]` keeps the bundler copying them.
#[used]
static APP_ICON_192: Asset = asset!(
    "/assets/brand/app-icon-192.png",
    AssetOptions::builder().with_hash_suffix(false)
);
#[used]
static APP_ICON_512: Asset = asset!(
    "/assets/brand/app-icon-512.png",
    AssetOptions::builder().with_hash_suffix(false)
);
#[used]
static APP_ICON_MASKABLE: Asset = asset!(
    "/assets/brand/app-icon-maskable-512.png",
    AssetOptions::builder().with_hash_suffix(false)
);
#[used]
static OUTFIT_FONT: Asset = asset!(
    "/assets/fonts/outfit-latin.woff2",
    AssetOptions::builder().with_hash_suffix(false)
);

/// Accessible name shared by every lockup image.
pub const BRAND_NAME: &str = "Wise Crow";

/// Which arrangement of symbol and wordmark to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lockup {
    /// Symbol beside the wordmark; navigation bars.
    Horizontal,
    /// Symbol above the wordmark; sign-in and empty states.
    Stacked,
}

impl Lockup {
    const fn images(self) -> (Asset, Asset) {
        match self {
            Self::Horizontal => (HORIZONTAL_INK, HORIZONTAL_CREAM),
            Self::Stacked => (STACKED_INK, STACKED_CREAM),
        }
    }
}

/// Renders a lockup that follows the colour theme.
#[component]
pub fn BrandLockup(lockup: Lockup, #[props(default)] class: String) -> Element {
    let (ink, cream) = lockup.images();
    rsx! {
        span { class: "brand-lockup {class}",
            img { class: "brand-light", src: ink, alt: BRAND_NAME }
            img { class: "brand-dark", src: cream, alt: BRAND_NAME }
        }
    }
}

/// Head elements for browser icons, the installable-app manifest and the
/// toolbar colour.
#[component]
pub fn BrandHead() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON_SVG, r#type: "image/svg+xml" }
        document::Link { rel: "alternate icon", href: FAVICON_ICO }
        document::Link { rel: "apple-touch-icon", href: APPLE_TOUCH_ICON }
        document::Link { rel: "manifest", href: WEB_MANIFEST }
        document::Meta { name: "theme-color", content: "#246B60" }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use dioxus::prelude::*;

    use super::{BrandLockup, Lockup, BRAND_NAME};

    fn lockup_fixture() -> Element {
        rsx! {
            BrandLockup { lockup: Lockup::Horizontal, class: "nav-brand" }
        }
    }

    #[test]
    fn lockup_ships_one_image_per_theme_with_the_brand_name() {
        let mut dom = VirtualDom::new(lockup_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);

        assert_eq!(html.matches(&format!("alt=\"{BRAND_NAME}\"")).count(), 2);
        assert!(html.contains("class=\"brand-light\"") && html.contains("class=\"brand-dark\""));
        assert!(html.contains("brand-lockup nav-brand"));
        assert!(
            html.contains("wise-crow-horizontal-ink")
                && html.contains("wise-crow-horizontal-cream")
        );
    }
}
