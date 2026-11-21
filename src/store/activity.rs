use super::*;

const MAX_ITEMS: usize = 64;
const ENTRIES: &str = "
    SELECT rowid AS position, 1 AS native, item FROM run_activity WHERE run_id=?1
    UNION ALL
    SELECT c.rowid, 0, json_object('id', c.item, 'kind', 'command',
           'title', '', 'status', CASE WHEN c.finished THEN 'completed' ELSE 'unknown' END)
    FROM command_output c WHERE c.run_id=?1 AND NOT EXISTS (
        SELECT 1 FROM run_activity a WHERE a.run_id=c.run_id AND a.item_id=c.item
    )";

pub(super) fn record(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    mut item: ActivityItem,
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
    if let Some(previous) = tx
        .query_row(
            "SELECT item FROM run_activity WHERE run_id=? AND item_id=?",
            params![run.0, item.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        let previous: ActivityItem = serde_json::from_str(&previous)?;
        if item.title.is_empty() {
            item.title = previous.title.clone();
        }
        if !matches!(
            previous.status,
            ActivityStatus::Running | ActivityStatus::Unknown
        ) || item == previous
        {
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
    tx.execute(
        "INSERT INTO run_activity(run_id,item_id,item,sequence) VALUES(?,?,?,?)
         ON CONFLICT(run_id,item_id) DO UPDATE SET item=excluded.item, sequence=excluded.sequence",
        params![run.0, item.id, serde_json::to_string(&item)?, sequence],
    )?;
    Ok(())
}

impl Database {
    pub(super) fn activity(&self, thread: &ThreadId, runs: &[&RunId]) -> Result<Vec<RunActivity>> {
        let mut status = self.db.prepare(
            "SELECT status,max(0,finished_at_ms-started_at_ms) FROM run WHERE id=? AND thread_id=?",
        )?;
        let mut items = self.db.prepare(&format!(
            "SELECT item FROM ({ENTRIES}) ORDER BY native DESC, position DESC LIMIT ?2"
        ))?;
        let classified = format!(
            "SELECT *, CASE json_extract(item,'$.kind')
                WHEN 'commentary' THEN 'message'
                WHEN 'compaction' THEN 'compaction'
                ELSE 'tool' END AS kind
             FROM ({ENTRIES})"
        );
        let mut live_items = self.db.prepare(&format!(
            "WITH ranked AS (
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
             SELECT item,total,tools,messages,failed FROM ranked
             WHERE rank<=?2 OR (kind_rank=1 AND kind<>'compaction') ORDER BY rank DESC"
        ))?;
        let mut total_counts = self.db.prepare(&format!(
            "SELECT count(*) FILTER (WHERE kind='tool'),
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
            Update::Activity(ActivityItem {
                id: "commentary".into(),
                kind: ActivityKind::Commentary,
                title: "Checking the patch".into(),
                status: ActivityStatus::Completed,
                timestamp_ms: None,
            }),
        )?;
        for index in 0..70 {
            db.update(
                &thread,
                &run,
                Update::Activity(ActivityItem {
                    id: format!("tool-{index}"),
                    kind: ActivityKind::Tool,
                    title: format!("Read file {index}"),
                    status: if index == 0 {
                        ActivityStatus::Failed
                    } else {
                        ActivityStatus::Completed
                    },
                    timestamp_ms: None,
                }),
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
        Ok(())
    }
}
