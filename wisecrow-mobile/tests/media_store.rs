//! The owner-keyed media store.
//!
//! A translation's clip and a sentence's clip are different things even when
//! their keys print the same; a clip belongs to the profile and user that
//! fetched it; and a sentence needs no translation row to be stored.

use chrono::Utc;
use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_dto::UserDto;
use wisecrow_mobile::application::{ContentRepository, MobileError, ProfileRepository};
use wisecrow_mobile::storage::models::{
    MediaEntry, MediaOwner, MediaRegistration, MediaType, Profile, ProfileIdentity,
};
use wisecrow_mobile::storage::SqliteStore;

type TestResult = Result<(), MobileError>;

fn identity(user_id: i32) -> ProfileIdentity {
    let now = Utc::now();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::new_v4(),
            // One server per learner here: a profile's origin is unique.
            origin: format!("https://media-{user_id}.example.test/"),
            imported_ca_fingerprint: None,
            active: true,
            created_at: now,
            updated_at: now,
        },
        user: UserDto {
            id: user_id,
            display_name: String::from("Test User"),
        },
        device_id: Uuid::new_v4(),
    }
}

fn registration(owner: MediaOwner, file_name: &str, bytes: u64) -> MediaRegistration {
    MediaRegistration {
        owner,
        media_type: MediaType::Audio,
        file_name: String::from(file_name),
        byte_length: bytes,
        attribution: None,
        fingerprint: Some("ab".repeat(32)),
        last_accessed_at: Utc::now(),
    }
}

fn sentence() -> MediaOwner {
    MediaOwner::Sentence("ab".repeat(32))
}

#[tokio::test]
async fn a_sentence_clip_needs_no_translation_and_keeps_its_fingerprint() -> TestResult {
    let dir = tempdir()?;
    let store = SqliteStore::open(&dir.path().join("media.sqlite3")).await?;
    store.save_profile_identity(&identity(7)).await?;
    let root = dir.path().join("media");
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("sentence-ab.mp3"), b"clip")?;

    store
        .register_media(&root, &registration(sentence(), "sentence-ab.mp3", 4))
        .await?;

    let entry: MediaEntry = store
        .media(&root, &sentence(), MediaType::Audio, Utc::now())
        .await?
        .expect("the clip is stored");
    assert_eq!(entry.owner, sentence());
    assert_eq!(entry.fingerprint.as_deref(), Some("ab".repeat(32).as_str()));
    assert_eq!(entry.byte_length, 4);
    assert!(
        store
            .media(
                &root,
                &MediaOwner::Translation(1),
                MediaType::Audio,
                Utc::now()
            )
            .await?
            .is_none(),
        "a translation with the same key text is a different owner"
    );
    Ok(())
}

#[tokio::test]
async fn a_translation_clip_still_requires_its_translation_row() -> TestResult {
    let dir = tempdir()?;
    let store = SqliteStore::open(&dir.path().join("media.sqlite3")).await?;
    store.save_profile_identity(&identity(7)).await?;
    let root = dir.path().join("media");
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("one.mp3"), b"one")?;

    let refused = store
        .register_media(
            &root,
            &registration(MediaOwner::Translation(1), "one.mp3", 3),
        )
        .await;
    assert!(
        matches!(refused, Err(MobileError::Conflict(_))),
        "no such translation offline"
    );
    Ok(())
}

#[tokio::test]
async fn clips_are_scoped_to_the_profile_and_user_that_fetched_them() -> TestResult {
    let dir = tempdir()?;
    let store = SqliteStore::open(&dir.path().join("media.sqlite3")).await?;
    let root = dir.path().join("media");
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("sentence-ab.mp3"), b"clip")?;

    let first = identity(7);
    store.save_profile_identity(&first).await?;
    store
        .register_media(&root, &registration(sentence(), "sentence-ab.mp3", 4))
        .await?;

    let second = identity(8);
    store.save_profile_identity(&second).await?;
    store.activate_profile(second.profile.id).await?;
    assert!(
        store
            .media(&root, &sentence(), MediaType::Audio, Utc::now())
            .await?
            .is_none(),
        "another profile does not see the first profile's clip"
    );
    assert!(store.media_lru(&root).await?.is_empty());

    store.activate_profile(first.profile.id).await?;
    assert_eq!(store.media_lru(&root).await?.len(), 1);
    store
        .confirm_media_deleted(&sentence(), MediaType::Audio)
        .await?;
    assert!(store.media_lru(&root).await?.is_empty());
    Ok(())
}
