//! Antigravity Google 账号池。
//!
//! 官方登录始终对应供应商 `antigravity-official`；账号是独立快照列表，
//! 不是 `providers` 表行。切换账号只恢复 token / Windows 凭据。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ACCOUNT_ID_PREFIX: &str = "antigravity-account-";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityAccount {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing)]
    pub auth_payload: Value,
    pub is_current: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Database {
    pub fn list_antigravity_accounts(&self) -> Result<Vec<AntigravityAccount>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, email, auth_payload, is_current, created_at, updated_at
                 FROM antigravity_accounts
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let payload_str: String = row.get(3)?;
                let auth_payload = serde_json::from_str(&payload_str).unwrap_or(Value::Null);
                Ok(AntigravityAccount {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    email: row.get(2)?,
                    auth_payload,
                    is_current: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(accounts)
    }

    pub fn get_antigravity_account(
        &self,
        id: &str,
    ) -> Result<Option<AntigravityAccount>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, name, email, auth_payload, is_current, created_at, updated_at
             FROM antigravity_accounts WHERE id = ?1",
            params![id],
            |row| {
                let payload_str: String = row.get(3)?;
                let auth_payload = serde_json::from_str(&payload_str).unwrap_or(Value::Null);
                Ok(AntigravityAccount {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    email: row.get(2)?,
                    auth_payload,
                    is_current: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(e.to_string()))
    }

    pub fn upsert_antigravity_account(
        &self,
        id: &str,
        name: &str,
        email: Option<&str>,
        auth_payload: &Value,
        set_current: bool,
    ) -> Result<(), AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let payload = serde_json::to_string(auth_payload)
            .map_err(|e| AppError::Database(format!("Failed to serialize auth payload: {e}")))?;
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        if set_current {
            tx.execute("UPDATE antigravity_accounts SET is_current = 0", [])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        tx.execute(
            "INSERT INTO antigravity_accounts
                (id, name, email, auth_payload, is_current, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                email = excluded.email,
                auth_payload = excluded.auth_payload,
                is_current = CASE WHEN ?5 = 1 THEN 1 ELSE antigravity_accounts.is_current END,
                updated_at = excluded.updated_at",
            params![
                id,
                name,
                email,
                payload,
                if set_current { 1 } else { 0 },
                now
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn set_current_antigravity_account(&self, id: &str) -> Result<(), AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        tx.execute("UPDATE antigravity_accounts SET is_current = 0", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        let updated = tx
            .execute(
                "UPDATE antigravity_accounts SET is_current = 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        if updated != 1 {
            return Err(AppError::Database(format!(
                "Antigravity account '{id}' not found"
            )));
        }
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn delete_antigravity_account(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM antigravity_accounts WHERE id = ?1",
            params![id],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 把上一会话误做成供应商的 `antigravity-account-*` 迁进账号池后删除。
    pub fn migrate_legacy_antigravity_account_providers(&self) -> Result<u32, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut stmt = tx
            .prepare(
                "SELECT id, name, settings_config, is_current, created_at
                 FROM providers
                 WHERE app_type = 'antigravity' AND id LIKE 'antigravity-account-%'",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        let legacy: Vec<_> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);

        if legacy.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().timestamp_millis();
        let mut migrated = 0u32;
        let mut had_current = false;
        for (id, name, settings_str, is_current, created_at) in &legacy {
            if !id.starts_with(ACCOUNT_ID_PREFIX) {
                continue;
            }
            let settings: Value = serde_json::from_str(settings_str).unwrap_or(Value::Null);
            let mut payload = serde_json::Map::new();
            if let Some(token) = settings.get("token") {
                payload.insert("token".to_string(), token.clone());
            }
            if let Some(credential) = settings.get("credential") {
                payload.insert("credential".to_string(), credential.clone());
            }
            if payload.is_empty() {
                continue;
            }
            let email = settings
                .get("token")
                .and_then(|t| t.get("token").or(Some(t)))
                .and_then(|t| t.get("email"))
                .and_then(Value::as_str)
                .or_else(|| settings.get("email").and_then(Value::as_str));
            let payload_json = serde_json::to_string(&Value::Object(payload))
                .map_err(|e| AppError::Database(format!("serialize payload: {e}")))?;
            tx.execute(
                "INSERT INTO antigravity_accounts
                    (id, name, email, auth_payload, is_current, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    email = excluded.email,
                    auth_payload = excluded.auth_payload,
                    is_current = CASE WHEN excluded.is_current = 1 THEN 1 ELSE antigravity_accounts.is_current END,
                    updated_at = excluded.updated_at",
                params![
                    id,
                    name,
                    email,
                    payload_json,
                    if *is_current { 1 } else { 0 },
                    created_at.unwrap_or(now),
                    now
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            if *is_current {
                had_current = true;
            }
            migrated += 1;
        }

        tx.execute(
            "DELETE FROM providers WHERE app_type = 'antigravity' AND id LIKE 'antigravity-account-%'",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        if had_current {
            tx.execute(
                "UPDATE providers SET is_current = 0 WHERE app_type = 'antigravity'",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            tx.execute(
                "UPDATE providers SET is_current = 1
                 WHERE app_type = 'antigravity' AND id = 'antigravity-official'",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(migrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upsert_list_switch_delete_account_pool() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.upsert_antigravity_account(
            "antigravity-account-aaa",
            "Alice",
            Some("alice@gmail.com"),
            &json!({"token": {"refresh_token": "r1"}}),
            true,
        )?;
        db.upsert_antigravity_account(
            "antigravity-account-bbb",
            "Bob",
            None,
            &json!({"token": {"refresh_token": "r2"}}),
            false,
        )?;
        let listed = db.list_antigravity_accounts()?;
        assert_eq!(listed.len(), 2);
        assert!(listed[0].is_current);
        assert!(!listed[1].is_current);

        db.set_current_antigravity_account("antigravity-account-bbb")?;
        let listed = db.list_antigravity_accounts()?;
        assert!(!listed[0].is_current);
        assert!(listed[1].is_current);

        db.delete_antigravity_account("antigravity-account-aaa")?;
        assert_eq!(db.list_antigravity_accounts()?.len(), 1);
        Ok(())
    }

    #[test]
    fn migrates_legacy_account_providers() -> Result<(), AppError> {
        let db = Database::memory()?;
        let mut alice = crate::provider::Provider::with_id(
            "antigravity-account-aaa".to_string(),
            "Alice".to_string(),
            json!({"authType": "oauth", "token": {"refresh_token": "r1"}}),
            None,
        );
        alice.created_at = Some(1);
        db.save_provider("antigravity", &alice)?;
        db.save_provider(
            "antigravity",
            &crate::provider::Provider::with_id(
                "antigravity-official".to_string(),
                "Antigravity Official".to_string(),
                json!({"authType": "oauth"}),
                None,
            ),
        )?;
        db.set_current_provider("antigravity", "antigravity-account-aaa")?;

        let migrated = db.migrate_legacy_antigravity_account_providers()?;
        assert_eq!(migrated, 1);
        assert!(db
            .get_provider_by_id("antigravity-account-aaa", "antigravity")?
            .is_none());
        let accounts = db.list_antigravity_accounts()?;
        assert_eq!(accounts.len(), 1);
        assert!(accounts[0].is_current);
        assert_eq!(
            db.get_current_provider("antigravity")?.as_deref(),
            Some("antigravity-official")
        );
        Ok(())
    }
}
