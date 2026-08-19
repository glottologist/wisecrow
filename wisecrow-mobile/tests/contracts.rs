use wisecrow_dto::{MobileSessionDto, UserDto};
use wisecrow_mobile::application::{AppServices, MobileApi};
use wisecrow_mobile::auth::AuthState;
use wisecrow_mobile::storage::SqliteStore;

#[test]
fn public_contracts_are_available() {
    let state = AuthState::Anonymous;
    assert!(matches!(state, AuthState::Anonymous));

    let session = MobileSessionDto {
        token: "secret".to_owned(),
        user: UserDto {
            id: 7,
            display_name: "Test".to_owned(),
        },
    };
    assert_eq!(session.user.id, 7);

    let _services_type = std::any::TypeId::of::<AppServices>();
    let _store_type = std::any::TypeId::of::<SqliteStore>();
    let _api_type = std::any::TypeId::of::<&dyn MobileApi>();
}

#[test]
fn mobile_protocol_contracts_are_available() {
    use wisecrow_dto::mobile;

    assert_eq!(mobile::MOBILE_PROTOCOL_VERSION, 1);
    let _ = std::any::TypeId::of::<mobile::MobileCapabilitiesDto>();
    let _ = std::any::TypeId::of::<mobile::ProtocolErrorDto>();
    let _ = std::any::TypeId::of::<mobile::DeviceRegistrationRequestDto>();
    let _ = std::any::TypeId::of::<mobile::RegisteredDeviceDto>();
    let _ = std::any::TypeId::of::<mobile::CorpusPageDto>();
    let _ = std::any::TypeId::of::<mobile::CorpusChangePageDto>();
    let _ = std::any::TypeId::of::<mobile::ReviewBatchRequestDto>();
    let _ = std::any::TypeId::of::<mobile::ReviewBatchResponseDto>();
    let _ = std::any::TypeId::of::<mobile::CardChangePageDto>();
    let _ = std::any::TypeId::of::<mobile::NbackBatchRequestDto>();
    let _ = std::any::TypeId::of::<mobile::NbackBatchResponseDto>();
    let _ = std::any::TypeId::of::<mobile::CachedQuizDto>();
}
