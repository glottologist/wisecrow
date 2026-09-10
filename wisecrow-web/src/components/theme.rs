// Rust guideline compliant 2026-09-10
use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "wisecrow-theme";
#[cfg(target_arch = "wasm32")]
const THEME_ATTRIBUTE: &str = "data-theme";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Theme {
    Light,
    #[default]
    Dark,
}

impl Theme {
    fn from_attribute(value: Option<&str>) -> Self {
        match value {
            Some("light") => Self::Light,
            _ => Self::Dark,
        }
    }

    #[cfg(target_arch = "wasm32")]
    const fn attribute_value(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[component]
pub fn ThemeProvider(children: Element) -> Element {
    let theme = use_signal(Theme::default);
    use_context_provider(move || theme);

    let mut hydrated_theme = theme;
    use_effect(move || hydrated_theme.set(read_browser_theme()));

    rsx! { {children} }
}

#[component]
pub fn ThemeSelector() -> Element {
    let selected_theme = use_context::<Signal<Theme>>();
    let current = selected_theme();

    rsx! {
        div {
            class: "theme-selector",
            role: "group",
            aria_label: "Colour theme",
            button {
                r#type: "button",
                class: "theme-option theme-option-light",
                aria_label: "Use light theme",
                aria_pressed: if current == Theme::Light { "true" } else { "false" },
                onclick: move |_| select_theme(selected_theme, Theme::Light),
                SunIcon {}
                span { "Light" }
            }
            button {
                r#type: "button",
                class: "theme-option theme-option-dark",
                aria_label: "Use dark theme",
                aria_pressed: if current == Theme::Dark { "true" } else { "false" },
                onclick: move |_| select_theme(selected_theme, Theme::Dark),
                MoonIcon {}
                span { "Dark" }
            }
        }
    }
}

fn select_theme(mut shared_theme: Signal<Theme>, theme: Theme) {
    shared_theme.set(theme);
    apply_browser_theme(theme);
}

#[component]
fn SunIcon() -> Element {
    rsx! {
        svg {
            class: "theme-icon",
            view_box: "0 0 20 20",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            "aria-hidden": "true",
            circle { cx: "10", cy: "10", r: "3.25" }
            path { d: "M10 1.5v2M10 16.5v2M1.5 10h2M16.5 10h2M4 4l1.4 1.4M14.6 14.6L16 16M16 4l-1.4 1.4M5.4 14.6L4 16" }
        }
    }
}

#[component]
fn MoonIcon() -> Element {
    rsx! {
        svg {
            class: "theme-icon",
            view_box: "0 0 20 20",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "M17 12.5A7 7 0 0 1 7.5 3a7 7 0 1 0 9.5 9.5Z" }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn document_root() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.document_element()
}

#[cfg(target_arch = "wasm32")]
fn read_browser_theme() -> Theme {
    let attribute = document_root().and_then(|root| root.get_attribute(THEME_ATTRIBUTE));
    Theme::from_attribute(attribute.as_deref())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_browser_theme() -> Theme {
    Theme::from_attribute(None)
}

#[cfg(target_arch = "wasm32")]
fn apply_browser_theme(theme: Theme) {
    if let Some(root) = document_root() {
        if let Err(error) = root.set_attribute(THEME_ATTRIBUTE, theme.attribute_value()) {
            tracing::warn!(?error, "failed to apply browser theme");
        }
    }

    let Some(window) = web_sys::window() else {
        return;
    };
    match window.local_storage() {
        Ok(Some(storage)) => {
            if let Err(error) = storage.set_item(STORAGE_KEY, theme.attribute_value()) {
                tracing::warn!(?error, "failed to persist browser theme");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(?error, "browser theme storage unavailable"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn apply_browser_theme(_theme: Theme) {}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;
    use rstest::rstest;

    use super::{Theme, ThemeProvider, ThemeSelector};

    #[rstest]
    #[case(Some("light"), Theme::Light)]
    #[case(Some("dark"), Theme::Dark)]
    #[case(None, Theme::Dark)]
    #[case(Some("sepia"), Theme::Dark)]
    fn root_attribute_parsing_is_bounded(#[case] attribute: Option<&str>, #[case] expected: Theme) {
        assert_eq!(Theme::from_attribute(attribute), expected);
    }

    #[cfg(feature = "server")]
    fn selector_fixture() -> Element {
        rsx! {
            ThemeProvider {
                ThemeSelector {}
            }
        }
    }

    #[test]
    fn stylesheet_covers_both_palettes_and_native_controls() {
        const STYLES: &str = include_str!("../../assets/style.css");
        let required = [
            ":root[data-theme=\"light\"]",
            "color-scheme: dark",
            "color-scheme: light",
            ".theme-selector",
            ".login-theme-selector",
            "input:-webkit-autofill",
            "scrollbar-color",
            ":root[data-theme=\"light\"] select",
        ];
        let missing: Vec<&str> = required
            .into_iter()
            .filter(|declaration| !STYLES.contains(declaration))
            .collect();

        assert!(
            missing.is_empty(),
            "missing theme CSS contracts: {missing:?}"
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn selector_exposes_labels_and_pressed_state() {
        let mut dom = VirtualDom::new(selector_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);

        assert!(
            html.contains("role=\"group\"")
                && html.contains("aria-label=\"Colour theme\"")
                && html.contains("Use light theme")
                && html.contains("Use dark theme")
                && html.contains("aria-pressed=\"true\"")
                && html.contains("aria-pressed=\"false\"")
        );
    }
}
