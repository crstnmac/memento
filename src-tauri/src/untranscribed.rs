//! Recordings that have audio on disk but no transcript yet (interrupted by a
//! crash and recovered at startup, or whose transcription failed). The UI
//! offers to transcribe them in one go.

use std::sync::Arc;

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::state::MonitorState;

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UntranscribedRecording {
    pub id: i64,
    pub title: String,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
}

fn query(conn: &Connection) -> Result<Vec<UntranscribedRecording>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, started_at, duration_ms FROM conversations
             WHERE audio_path IS NOT NULL AND TRIM(audio_path) != ''
               AND (transcript IS NULL OR TRIM(transcript) = '')
             ORDER BY started_at DESC, id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(UntranscribedRecording {
                id: row.get(0)?,
                title: row.get(1)?,
                started_at: row.get(2)?,
                duration_ms: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_untranscribed_recordings(
    state: State<'_, Arc<MonitorState>>,
) -> Result<Vec<UntranscribedRecording>, String> {
    query(&state.db().conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_audio_without_transcript_newest_first() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE conversations (id INTEGER PRIMARY KEY, title TEXT, transcript TEXT,
               audio_path TEXT, started_at INTEGER, duration_ms INTEGER);
             INSERT INTO conversations VALUES (1, 'old', NULL, '/a.wav', 100, 5);
             INSERT INTO conversations VALUES (2, 'done', 'hello', '/b.wav', 200, 5);
             INSERT INTO conversations VALUES (3, 'no audio', NULL, NULL, 300, NULL);
             INSERT INTO conversations VALUES (4, 'new', '  ', '/c.wav', 400, NULL);",
        )
        .unwrap();
        let rows = query(&conn).unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![4, 1]);
        assert_eq!(rows[1].duration_ms, Some(5));
    }
}
