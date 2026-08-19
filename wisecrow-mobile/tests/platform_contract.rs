use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_mobile::{
    application::{
        BackgroundScheduler, CertificateStore, CredentialStore, FilePicker, MobileError,
    },
    platform::{DesktopPlatform, PlatformTransport},
};

fn assert_platform_contract<T>()
where
    T: CredentialStore + CertificateStore + FilePicker + BackgroundScheduler + PlatformTransport,
{
}

#[tokio::test]
async fn desktop_platform_implements_the_native_boundary() {
    assert_platform_contract::<DesktopPlatform>();
    let directory = tempdir().expect("temporary directory");
    let platform = DesktopPlatform::new(directory.path()).expect("desktop platform");
    let profile_id = Uuid::from_u128(1);

    assert!(matches!(
        platform.pick_pdf(1_024).await,
        Err(MobileError::Unsupported)
    ));
    assert!(matches!(
        platform.pick_certificate(1_024).await,
        Err(MobileError::Unsupported)
    ));
    assert!(matches!(
        platform.schedule_sync(profile_id).await,
        Err(MobileError::Unsupported)
    ));
    assert!(matches!(
        platform.cancel_sync(profile_id).await,
        Err(MobileError::Unsupported)
    ));
}
