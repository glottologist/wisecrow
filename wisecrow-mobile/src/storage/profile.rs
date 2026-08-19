use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Sqlite, Transaction};
use uuid::Uuid;
use wisecrow_dto::UserDto;

use super::{
    models::{Profile, ProfileIdentity},
    SqliteStore,
};
use crate::application::{MobileError, ProfileRepository};

#[derive(FromRow)]
struct ProfileRow {
    id: Uuid,
    origin: String,
    imported_ca_fingerprint: Option<String>,
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ProfileRow {
    fn into_profile(self) -> Profile {
        Profile {
            id: self.id,
            origin: self.origin,
            imported_ca_fingerprint: self.imported_ca_fingerprint,
            active: self.active,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(FromRow)]
struct IdentityRow {
    id: Uuid,
    origin: String,
    imported_ca_fingerprint: Option<String>,
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    user_id: i32,
    display_name: String,
    device_id: Uuid,
}

impl IdentityRow {
    fn into_identity(self) -> ProfileIdentity {
        ProfileIdentity {
            profile: Profile {
                id: self.id,
                origin: self.origin,
                imported_ca_fingerprint: self.imported_ca_fingerprint,
                active: self.active,
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
            user: UserDto {
                id: self.user_id,
                display_name: self.display_name,
            },
            device_id: self.device_id,
        }
    }
}

#[async_trait]
impl ProfileRepository for SqliteStore {
    async fn active_profile(&self) -> Result<Option<Profile>, MobileError> {
        let row = sqlx::query_as::<_, ProfileRow>(
            "SELECT id, origin, imported_ca_fingerprint, active, created_at, updated_at
             FROM profiles WHERE active = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(ProfileRow::into_profile))
    }

    async fn active_identity(&self) -> Result<Option<ProfileIdentity>, MobileError> {
        let row = sqlx::query_as::<_, IdentityRow>(
            "SELECT p.id, p.origin, p.imported_ca_fingerprint, p.active,
                    p.created_at, p.updated_at, u.user_id, u.display_name, u.device_id
             FROM profiles p
             JOIN profile_users u
               ON u.profile_id = p.id AND u.user_id = p.active_user_id
             WHERE p.active = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(IdentityRow::into_identity))
    }

    async fn save_profile(&self, profile: &Profile) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        prepare_activation(&mut transaction, profile.active).await?;
        upsert_profile(&mut transaction, profile).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn activate_profile(&self, profile_id: Uuid) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        deactivate_profiles(&mut transaction).await?;
        let result = sqlx::query("UPDATE profiles SET active = 1 WHERE id = ?")
            .bind(profile_id)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound.into());
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn profile_identity(
        &self,
        profile_id: Uuid,
        user_id: i32,
    ) -> Result<Option<ProfileIdentity>, MobileError> {
        let row = sqlx::query_as::<_, IdentityRow>(
            "SELECT p.id, p.origin, p.imported_ca_fingerprint, p.active,
                    p.created_at, p.updated_at, u.user_id, u.display_name, u.device_id
             FROM profiles p
             JOIN profile_users u ON u.profile_id = p.id
             WHERE p.id = ? AND u.user_id = ?",
        )
        .bind(profile_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(IdentityRow::into_identity))
    }

    async fn save_profile_identity(&self, identity: &ProfileIdentity) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        prepare_activation(&mut transaction, identity.profile.active).await?;
        upsert_profile(&mut transaction, &identity.profile).await?;
        upsert_user(&mut transaction, identity).await?;
        set_active_user(&mut transaction, identity).await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn prepare_activation(
    transaction: &mut Transaction<'_, Sqlite>,
    active: bool,
) -> Result<(), sqlx::Error> {
    if active {
        deactivate_profiles(transaction).await?;
    }
    Ok(())
}

async fn deactivate_profiles(transaction: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE profiles SET active = 0 WHERE active = 1")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upsert_profile(
    transaction: &mut Transaction<'_, Sqlite>,
    profile: &Profile,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO profiles
             (id, origin, imported_ca_fingerprint, active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
             origin = excluded.origin,
             imported_ca_fingerprint = excluded.imported_ca_fingerprint,
             active = excluded.active,
             updated_at = excluded.updated_at",
    )
    .bind(profile.id)
    .bind(&profile.origin)
    .bind(&profile.imported_ca_fingerprint)
    .bind(profile.active)
    .bind(profile.created_at)
    .bind(profile.updated_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn upsert_user(
    transaction: &mut Transaction<'_, Sqlite>,
    identity: &ProfileIdentity,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO profile_users (profile_id, user_id, display_name, device_id)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(profile_id, user_id) DO UPDATE SET
             display_name = excluded.display_name,
             device_id = excluded.device_id",
    )
    .bind(identity.profile.id)
    .bind(identity.user.id)
    .bind(&identity.user.display_name)
    .bind(identity.device_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn set_active_user(
    transaction: &mut Transaction<'_, Sqlite>,
    identity: &ProfileIdentity,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query("UPDATE profiles SET active_user_id = ? WHERE id = ?")
        .bind(identity.user.id)
        .bind(identity.profile.id)
        .execute(&mut **transaction)
        .await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}
