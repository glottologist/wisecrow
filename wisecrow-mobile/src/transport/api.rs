use super::ServerOrigin;

/// Versioned HTTP implementation of the mobile server protocol.
pub struct HttpMobileApi {
    origin: ServerOrigin,
}

impl HttpMobileApi {
    #[must_use]
    pub fn new(origin: &ServerOrigin) -> Self {
        Self {
            origin: origin.clone(), // clone: the client owns its immutable normalized server origin
        }
    }

    #[must_use]
    pub fn origin(&self) -> &ServerOrigin {
        &self.origin
    }
}
