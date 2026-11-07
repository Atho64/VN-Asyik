//! Bounded local profile storage. No sync, export, secrets or inference.
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use vn_core::companion::profile::{Preset, ProfileSettings, ProfileSnapshot};

pub const MAX_PROFILES: i64 = 100;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    pub presets: Vec<ProfileSnapshot>,
    pub profiles: Vec<ProfileSnapshot>,
}

fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<ProfileSnapshot, String> {
    let profile = ProfileSnapshot {
        id: row.get("id"),
        version: row.get("version"),
        settings: serde_json::from_str(row.get("profile_json")).map_err(|_| "invalid_profile")?,
    };
    profile.validate()?;
    Ok(profile)
}

pub async fn list(pool: &SqlitePool) -> Result<Catalogue, String> {
    let rows = sqlx::query("SELECT id,version,profile_json FROM companion_profiles ORDER BY updated_at DESC,id LIMIT 100")
        .fetch_all(pool).await.map_err(|_| "db_error")?;
    Ok(Catalogue {
        presets: Preset::ALL.into_iter().map(Preset::snapshot).collect(),
        profiles: rows.iter().map(decode).collect::<Result<_, _>>()?,
    })
}

/// Updates require the version the editor opened. Another window cannot lose an edit.
pub async fn save(
    pool: &SqlitePool,
    id: Option<&str>,
    expected_version: Option<i32>,
    settings: ProfileSettings,
) -> Result<ProfileSnapshot, String> {
    settings.validate()?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    let now = crate::ai::conversation::store::now();
    let json = serde_json::to_string(&settings).map_err(|_| "invalid_profile")?;
    let (id, version) = if let Some(id) = id {
        uuid::Uuid::parse_str(id).map_err(|_| "invalid_profile")?;
        let version = expected_version
            .filter(|v| *v > 0)
            .and_then(|v| v.checked_add(1))
            .ok_or("profile_conflict")?;
        let result = sqlx::query("UPDATE companion_profiles SET version=?,profile_json=?,updated_at=? WHERE id=? AND version=?")
            .bind(version).bind(&json).bind(now).bind(id).bind(expected_version)
            .execute(&mut *tx).await.map_err(|_| "db_error")?;
        if result.rows_affected() != 1 {
            return Err("profile_conflict".into());
        }
        (id.to_owned(), version)
    } else {
        if expected_version.is_some() {
            return Err("invalid_profile".into());
        }
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM companion_profiles")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "db_error")?;
        if count >= MAX_PROFILES {
            return Err("profile_limit".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO companion_profiles(id,version,profile_json,created_at,updated_at) VALUES(?,1,?,?,?)")
            .bind(&id).bind(json).bind(now).bind(now).execute(&mut *tx).await.map_err(|_| "db_error")?;
        (id, 1)
    };
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(ProfileSnapshot {
        id,
        version,
        settings,
    })
}

pub async fn delete(pool: &SqlitePool, id: &str, version: i32) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM companion_profiles WHERE id=? AND version=?")
        .bind(id)
        .bind(version)
        .execute(pool)
        .await
        .map_err(|_| "db_error")?;
    if result.rows_affected() != 1 {
        return Err("profile_conflict".into());
    }
    Ok(())
}
