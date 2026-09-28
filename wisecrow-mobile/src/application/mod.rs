pub mod error;
pub mod grammar;
pub mod profile;
pub mod services;
pub mod traits;

pub use error::MobileError;
pub use grammar::{
    GrammarCursors, LocalGrammarItem, LocalGrammarMastery, LocalGrammarRule, LocalRuleExample,
    MissingExampleAudio, QueuedAttempt,
};
pub use profile::ProfileService;
pub use services::AppServices;
pub use traits::{
    ApiFactory, BackgroundScheduler, CertificateStore, ContentRepository, CorpusRepository,
    CredentialStore, FilePicker, GrammarRepository, LearningRepository, LocalStore, MobileApi,
    ProfileRepository,
};

/// The directory fetched media lives in, as the pages find it in context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRoot(pub std::path::PathBuf);
