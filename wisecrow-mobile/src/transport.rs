use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileConfig {
    pub(crate) origin: Url,
    pub(crate) dioxus_origin: &'static str,
}
