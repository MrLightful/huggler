use super::*;
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 64;
const TRACE_EXCERPT_BYTES: usize = 16 * 1024;
const SUMMARY: &str = "json_remove(item,'$.input','$.result')";
const TRACE_SOURCE: &str = "coalesce(json_extract(item,'$.source'),
    CASE json_extract(item,'$.kind') WHEN 'tool' THEN 'unknown' ELSE 'standard' END)";
const ENTRIES: &str = "
    SELECT a.rowid AS position, 1 AS native, a.item, r.id AS run_id, r.status, a.item_id
    FROM selected_runs r JOIN run_activity a ON a.run_id=r.id
    UNION ALL
    SELECT c.rowid, 0, json_object('id', c.item, 'kind', 'command',
           'title', '', 'status', CASE WHEN c.finished THEN 'completed' ELSE 'unknown' END),
           r.id, r.status, c.item
    FROM selected_runs r JOIN command_output c ON c.run_id=r.id WHERE NOT EXISTS (
        SELECT 1 FROM run_activity a WHERE a.run_id=c.run_id AND a.item_id=c.item
    )";
const ONE_RUN: &str = "WITH selected_runs AS (SELECT id,status FROM run WHERE id=?1)";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct StoredActivity {
    #[serde(flatten)]
    item: ActivityItem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<ToolTraceSource>,
    #[serde(flatten)]
    payload: ToolTracePayload,
}

pub(super) fn record(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    mut item: ActivityItem,
    mut source: Option<ToolTraceSource>,
    mut payload: ToolTracePayload,
) -> Result<()> {
    if item.id.is_empty() || item.id.len() > 512 {
        return Ok(());
    }
    let limit = if item.kind == ActivityKind::Commentary {
        4096
    } else {
        512
    };
    item.title.truncate(item.title.floor_char_boundary(limit));
    for text in [&mut payload.input, &mut payload.result]
        .into_iter()
        .flatten()
    {
        if text.len() > TRACE_EXCERPT_BYTES {
            text.truncate(text.floor_char_boundary(TRACE_EXCERPT_BYTES - '…'.len_utf8()));
            text.push('…');
        }
    }
    if let Some(previous) = tx
        .query_row(
            "SELECT item FROM run_activity WHERE run_id=? AND item_id=?",
            params![run.0, item.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        let previous: StoredActivity = serde_json::from_str(&previous)?;
        if !matches!(
            previous.item.status,
            ActivityStatus::Running | ActivityStatus::Unknown
        ) {
            // ACP can send content and status in separate patches.
            item = previous.item.clone();
        }
        if item.title.is_empty() {
            item.title = previous.item.title.clone();
        }
        if item.status == ActivityStatus::Unknown {
            item.status = previous.item.status;
        }
        source = if previous.source == Some(ToolTraceSource::Mcp) {
            previous.source
        } else {
            source.or(previous.source)
        };
        payload.input = payload.input.or_else(|| previous.payload.input.clone());
        payload.result = payload.result.or_else(|| previous.payload.result.clone());
        if item == previous.item && source == previous.source && payload == previous.payload {
            return Ok(());
        }
    }
    let sequence = event(
        tx,
        thread,
        Change::ActivityChanged {
            run_id: run.clone(),
        },
    )?;
    if item.kind == ActivityKind::Commentary {
        item.timestamp_ms = Some(tx.query_row(
            "SELECT CAST(unixepoch('subsec')*1000 AS INTEGER)",
            [],
            |row| row.get(0),
        )?);
    }
    let id = item.id.clone();
    let record = StoredActivity {
        item,
        source,
        payload,
    };
    tx.execute(
        "INSERT INTO run_activity(run_id,item_id,item,sequence) VALUES(?,?,?,?)
         ON CONFLICT(run_id,item_id) DO UPDATE SET item=excluded.item, sequence=excluded.sequence",
        params![run.0, id, serde_json::to_string(&record)?, sequence],
    )?;
    Ok(())
}

impl Database {
    pub fn tool_traces_changed_since(&self, thread: &ThreadId, after: i64) -> Result<bool> {
        self.db
            .query_row(
                "SELECT replay_floor > ?2 OR EXISTS (
                     SELECT 1 FROM event WHERE thread_id=?1 AND sequence>?2
                     AND json_extract(change,'$.type') IN ('activity_changed','command_output_available','run')
                 )
                 FROM thread WHERE id=?1",
                params![thread.0, after],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(thread.to_string()))
    }

    pub fn tool_traces(
        &self,
        thread: &ThreadId,
        source: Option<ToolTraceSource>,
        limit: usize,
    ) -> Result<Vec<ToolTrace>> {
        self.thread_position(thread)?;
        let source = source.map(serde_json::to_value).transpose()?;
        let source = source.as_ref().and_then(serde_json::Value::as_str);
        let mut run_query = self.db.prepare(
            "SELECT id FROM run WHERE thread_id=?1 ORDER BY coalesce(delivery_slot,ordinal) DESC",
        )?;
        let mut runs = run_query.query([&thread.0])?;
        let mut query = self.db.prepare(&format!(
            "{ONE_RUN}
             SELECT run_id,status,{TRACE_SOURCE},{SUMMARY}
             FROM ({ENTRIES})
             WHERE json_extract(item,'$.kind') NOT IN ('commentary','compaction')
                 AND (?2 IS NULL OR {TRACE_SOURCE}=?2)
             ORDER BY native DESC,position DESC LIMIT ?3"
        ))?;
        let mut traces = Vec::new();
        while traces.len() < limit
            && let Some(run) = runs.next()?
        {
            let run_id: String = run.get(0)?;
            let remaining = i64::try_from(limit - traces.len()).unwrap_or(i64::MAX);
            let mut rows = query.query(params![run_id, source, remaining])?;
            while let Some(row) = rows.next()? {
                traces.push(trace_row(row)?.0);
            }
        }
        Ok(traces)
    }

    pub fn tool_trace(
        &self,
        thread: &ThreadId,
        run: &RunId,
        item: &str,
    ) -> Result<(ToolTrace, ToolTracePayload)> {
        let mut query = self.db.prepare(&format!(
            "WITH selected_runs AS (SELECT id,status FROM run WHERE id=?1 AND thread_id=?2)
             SELECT run_id,status,{TRACE_SOURCE},item FROM ({ENTRIES})
             WHERE item_id=?3
                 AND json_extract(item,'$.kind') NOT IN ('commentary','compaction')"
        ))?;
        let mut rows = query.query(params![run.0, thread.0, item])?;
        trace_row(
            rows.next()?
                .ok_or_else(|| Error::NotFound("tool trace".into()))?,
        )
    }

    pub(super) fn activity(&self, thread: &ThreadId, runs: &[&RunId]) -> Result<Vec<RunActivity>> {
        let mut status = self.db.prepare(
            "SELECT status,max(0,finished_at_ms-started_at_ms) FROM run WHERE id=? AND thread_id=?",
        )?;
        let mut items = self.db.prepare(&format!(
            "{ONE_RUN} SELECT {SUMMARY} FROM ({ENTRIES}) ORDER BY native DESC, position DESC LIMIT ?2"
        ))?;
        let classified = format!(
            "SELECT *, CASE json_extract(item,'$.kind')
                WHEN 'commentary' THEN 'message'
                WHEN 'compaction' THEN 'compaction'
                ELSE 'tool' END AS kind
             FROM ({ENTRIES})"
        );
        let mut live_items = self.db.prepare(&format!(
            "{ONE_RUN}, ranked AS (
                SELECT item, kind,
                    row_number() OVER recent AS rank,
                    row_number() OVER (PARTITION BY kind ORDER BY native DESC,position DESC) AS kind_rank,
                    count(*) OVER () AS total,
                    sum(kind='tool') OVER () AS tools,
                    sum(kind='message') OVER () AS messages,
                    sum(kind='tool' AND json_extract(item,'$.status')='failed') OVER () AS failed
                FROM ({classified})
                WINDOW recent AS (ORDER BY native DESC,position DESC)
             )
             SELECT {SUMMARY},total,tools,messages,failed FROM ranked
             WHERE rank<=?2 OR (kind_rank=1 AND kind<>'compaction') ORDER BY rank DESC"
        ))?;
        let mut total_counts = self.db.prepare(&format!(
            "{ONE_RUN} SELECT count(*) FILTER (WHERE kind='tool'),
                    count(*) FILTER (WHERE kind='message'),
                    count(*) FILTER (WHERE kind='tool' AND json_extract(item,'$.status')='failed')
             FROM ({classified})"
        ))?;
        let mut activity = Vec::new();
        for run in runs {
            let Some((status, turn_duration_ms)) = status
                .query_row(params![run.0, thread.0], |row| {
                    Ok((row.get::<_, String>(0)?, row.get(1)?))
                })
                .optional()?
            else {
                continue;
            };
            let status: RunStatus = serde_json::from_value(status.into())?;
            let (records, truncated, counts) = if status.producing_output() {
                let mut rows = live_items.query(params![run.0, MAX_ITEMS as i64])?;
                let mut records = Vec::new();
                let mut total: i64 = 0;
                let mut counts = None;
                while let Some(row) = rows.next()? {
                    if counts.is_none() {
                        total = row.get(1)?;
                        counts = Some(ActivityCounts {
                            tools: row.get(2)?,
                            messages: row.get(3)?,
                            failed: row.get(4)?,
                        });
                    }
                    records.push(serde_json::from_str::<ActivityItem>(
                        &row.get::<_, String>(0)?,
                    )?);
                }
                let truncated = total > records.len() as i64;
                (records, truncated, counts)
            } else {
                let mut records = items
                    .query_map(params![run.0, MAX_ITEMS as i64 + 1], |row| {
                        row.get::<_, String>(0)
                    })?
                    .map(|row| Ok(serde_json::from_str::<ActivityItem>(&row?)?))
                    .collect::<Result<Vec<_>>>()?;
                let truncated = records.len() > MAX_ITEMS;
                records.truncate(MAX_ITEMS);
                records.reverse();
                let counts = if truncated {
                    Some(total_counts.query_row([&run.0], |row| {
                        Ok(ActivityCounts {
                            tools: row.get(0)?,
                            messages: row.get(1)?,
                            failed: row.get(2)?,
                        })
                    })?)
                } else {
                    None
                };
                (records, truncated, counts)
            };
            if !records.is_empty() || matches!(status, RunStatus::Failed | RunStatus::Interrupted) {
                activity.push(RunActivity {
                    run_id: (*run).clone(),
                    status,
                    turn_duration_ms,
                    counts,
                    items: records,
                    truncated,
                });
            }
        }
        Ok(activity)
    }
}

fn trace_row(row: &rusqlite::Row<'_>) -> Result<(ToolTrace, ToolTracePayload)> {
    let stored: StoredActivity = serde_json::from_str(&row.get::<_, String>(3)?)?;
    Ok((
        ToolTrace {
            run_id: RunId(row.get(0)?),
            run_status: serde_json::from_value(row.get::<_, String>(1)?.into())?,
            source: serde_json::from_value(row.get::<_, String>(2)?.into())?,
            item: stored.item,
        },
        stored.payload,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_activity_counts_survive_turn_completion() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut db = Database::open(directory.path())?;
        let project = db.project(directory.path())?;
        let thread = ThreadId("thread".into());
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: thread.clone(),
            action: Action::Launch {
                project: project.id,
                name: "Review".into(),
                model: None,
            },
        })?;
        let (receipt, _) = db.dispatch(&Command {
            id: CommandId("send".into()),
            thread_id: thread.clone(),
            action: Action::Send {
                text: "Review the patch".into(),
            },
        })?;
        let run = receipt.run_id.unwrap();
        db.update(
            &thread,
            &run,
            Update::Activity(
                ActivityItem {
                    id: "commentary".into(),
                    kind: ActivityKind::Commentary,
                    title: "Checking the patch".into(),
                    status: ActivityStatus::Completed,
                    timestamp_ms: None,
                },
                None,
                Default::default(),
            ),
        )?;
        for index in 0..70 {
            db.update(
                &thread,
                &run,
                Update::Activity(
                    ActivityItem {
                        id: format!("tool-{index}"),
                        kind: ActivityKind::Tool,
                        title: format!("Read file {index}"),
                        status: if index == 0 {
                            ActivityStatus::Failed
                        } else {
                            ActivityStatus::Completed
                        },
                        timestamp_ms: None,
                    },
                    None,
                    Default::default(),
                ),
            )?;
        }

        let detail = db.detail(&thread, None, "", 32)?;
        assert!(detail.foreground_run.unwrap().started_at_ms.is_some());
        let activity = &detail.activity[0];
        assert_eq!(
            activity.counts,
            Some(ActivityCounts {
                tools: 70,
                messages: 1,
                failed: 1
            })
        );
        assert!(activity.truncated);
        assert_eq!(activity.items.len(), MAX_ITEMS + 1);
        assert_eq!(activity.items.first().unwrap().id, "commentary");
        assert_eq!(activity.items.last().unwrap().id, "tool-69");
        db.update(&thread, &run, Update::Started(None))?;
        let cursor = db.sequence(&thread)?;
        db.update(&thread, &run, Update::Output("Review in progress".into()))?;
        assert!(!db.tool_traces_changed_since(&thread, cursor)?);
        db.update(
            &thread,
            &run,
            Update::Finalizing("Checked the patch".into()),
        )?;
        db.finish(&thread, &run, Ok(()))?;
        let settled = db.detail(&thread, None, "", 32)?;
        let saved = &settled.activity[0];
        assert_eq!(saved.status, RunStatus::Completed);
        assert_eq!(saved.counts, activity.counts);
        assert!(saved.truncated);
        assert_eq!(saved.items.len(), MAX_ITEMS);
        assert_eq!(saved.items.first().unwrap().id, "tool-6");
        assert_eq!(saved.items.last().unwrap().id, "tool-69");
        let traces = db.tool_traces(&thread, None, 71)?;
        assert_eq!(traces.len(), 70);
        assert_eq!(traces.last().unwrap().item.id, "tool-0");
        assert_eq!(traces.last().unwrap().item.status, ActivityStatus::Failed);
        assert_eq!(traces[0].source, ToolTraceSource::Unknown);
        assert!(
            db.tool_traces(&thread, Some(ToolTraceSource::Mcp), 71)?
                .is_empty()
        );
        Ok(())
    }
}
