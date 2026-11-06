use super::service::Work;
use crate::ai::conversation::{ChatMessage, Source, store as chat};
use sqlx::SqlitePool;

pub(super) async fn begin(pool: &SqlitePool, work: &Work) -> Result<ChatMessage, String> {
    let thread = chat::get(pool, &work.session.thread_id).await?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    crate::companion::memory::check_revision(&mut tx, work.memory.revision).await?;
    chat::trim_window(
        &mut tx,
        &thread.id,
        if work.automatic { 1 } else { 2 },
        work.memory.capture.as_ref(),
    )
    .await?;
    let mut pos = chat::last_position(&mut tx, &thread.id).await?;
    if !work.automatic {
        sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'user',?,'complete',?)")
            .bind(uuid::Uuid::new_v4().to_string()).bind(&thread.id).bind(pos).bind(&work.text).bind(chat::now())
            .execute(&mut *tx).await.map_err(|_|"db_error")?;
        pos += 1;
    }
    // An automatic observation has only an assistant row, never a forged reader question.
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'assistant','','running',?)")
        .bind(&id).bind(&thread.id).bind(pos).bind(chat::now()).execute(&mut *tx).await.map_err(|_|"db_error")?;
    let origin = if work.automatic {
        "automatic_reaction"
    } else {
        "manual_image"
    };
    sqlx::query("INSERT INTO companion_visual_turns(message_id,session_id,playthrough_id,origin,generation,sequence,captured_at,width,height,crop_json,knowledge,provider,model,status,description) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,'running','Visual inference; image no longer retained in history')")
        .bind(&id).bind(&work.session.id).bind(&work.session.playthrough_id).bind(origin)
        .bind(i64::try_from(work.generation).map_err(|_|"vision_unavailable")?).bind(i64::try_from(work.scene).map_err(|_|"vision_unavailable")?)
        .bind(work.frame.at).bind(work.frame.width).bind(work.frame.height).bind(serde_json::to_string(&work.crop).map_err(|_|"vision_invalid_crop")?)
        .bind(if work.session.options.knowledge==vn_core::companion::Knowledge::All{"all"}else{"observed"})
        .bind(work.provider.config.provider.id()).bind(&work.provider.config.model).execute(&mut *tx).await.map_err(|_|"db_error")?;
    let message = ChatMessage {
        id,
        role: "assistant".into(),
        content: String::new(),
        status: "running".into(),
        error: None,
        tools: Vec::new(),
        sources: vec![Source {
            id: origin.into(),
            title: "Visual inference; image not retained".into(),
            url: None,
            game_id: None,
        }],
        tokens: None,
        created_at: chat::now(),
    };
    sqlx::query("UPDATE chat_messages SET sources_json=? WHERE id=?")
        .bind(serde_json::to_string(&message.sources).map_err(|_| "db_error")?)
        .bind(&message.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    sqlx::query("UPDATE chat_threads SET updated_at=? WHERE id=?")
        .bind(chat::now())
        .bind(&thread.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(message)
}
pub(super) async fn save(
    pool: &SqlitePool,
    message: &ChatMessage,
    silent: bool,
) -> Result<(), String> {
    chat::save(pool, message).await?;
    sqlx::query("UPDATE companion_visual_turns SET status=? WHERE message_id=?")
        .bind(if silent { "silent" } else { &message.status })
        .bind(&message.id)
        .execute(pool)
        .await
        .map_err(|_| "db_error")?;
    Ok(())
}
