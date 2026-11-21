//! Durable time and webhook triggers driven by the embedding host.
use crate::*;
use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trigger {
    Once {
        at: i64,
    },
    Interval {
        first: i64,
        seconds: u32,
    },
    /// One occurrence per local day. Skip nonexistent times and use the earlier offset on a fold.
    Daily {
        zone: String,
        hour: u8,
        minute: u8,
    },
    Webhook {
        event: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Destination {
    Continue,
    FreshSession,
    NewThread,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub trigger: Trigger,
    pub destination: Destination,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub key: String,
    pub thread: ThreadId,
    pub run: RunId,
}

impl Trigger {
    pub(crate) fn next(&self, after: i64) -> Result<Option<i64>> {
        let invalid = || Error::Invalid("invalid schedule time or time zone".into());
        Ok(match self {
            Self::Once { at } => (*at > after).then_some(*at),
            Self::Interval { first, seconds } => {
                if *seconds == 0 {
                    return Err(invalid());
                }
                let step = i128::from(*seconds);
                let elapsed = i128::from(after) - i128::from(*first);
                let n = if elapsed < 0 { 0 } else { elapsed / step + 1 };
                Some(
                    (i128::from(*first) + n * step)
                        .try_into()
                        .map_err(|_| invalid())?,
                )
            }
            Self::Daily { zone, hour, minute } => {
                let tz: Tz = zone.parse().map_err(|_| invalid())?;
                let start = DateTime::from_timestamp(after, 0).ok_or_else(invalid)?;
                let mut date = start.with_timezone(&tz).date_naive();
                loop {
                    let local = date
                        .and_hms_opt((*hour).into(), (*minute).into(), 0)
                        .ok_or_else(invalid)?;
                    if let Some(next) = tz.from_local_datetime(&local).earliest()
                        && next.timestamp() > after
                    {
                        break Some(next.timestamp());
                    }
                    date = date.succ_opt().ok_or_else(invalid)?;
                }
            }
            Self::Webhook { .. } => None,
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct Delivery {
    pub id: String,
    pub event: String,
}

impl Huggler {
    /// The host retains the signing key for webhook schedules. Times are Unix seconds.
    pub fn create_schedule(
        &self,
        owner: &ThreadId,
        spec: Schedule,
        now: i64,
        signing_key: Option<&[u8]>,
    ) -> Result<String> {
        self.shared
            .access(|db| db.create_schedule(owner, &spec, now, signing_key))
    }

    pub fn schedules(&self, thread: &ThreadId) -> Result<Vec<(String, Schedule, Option<i64>)>> {
        self.shared.access(|db| db.schedules(thread))
    }

    /// Release one due occurrence, coalescing missed intervals. Safe to call concurrently.
    pub fn tick_schedule(&self, id: &str, now: i64) -> Result<Option<Occurrence>> {
        self.shared.schedule(|db| db.tick_schedule(id, now))
    }

    /// Verify the exact raw JSON bytes before interpreting the signed event and delivery ID.
    /// The host supplies its configured key. The signature is hex HMAC-SHA256.
    pub fn deliver_webhook(
        &self,
        id: &str,
        signing_key: &[u8],
        signature: &str,
        body: &[u8],
    ) -> Result<Option<Occurrence>> {
        if body.len() > 64 * 1024 {
            return Err(Error::Invalid("webhook exceeds 64 KiB".into()));
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(signing_key).map_err(|_| Error::Forbidden)?;
        mac.update(body);
        let signature = hex::decode(signature).map_err(|_| Error::Forbidden)?;
        mac.verify_slice(&signature).map_err(|_| Error::Forbidden)?;
        let delivery: Delivery = serde_json::from_slice(body)?;
        self.shared
            .schedule(|db| db.deliver_webhook(id, signing_key, delivery))
    }
    pub fn schedule_history(&self, id: &str) -> Result<Vec<Occurrence>> {
        self.shared.access(|db| db.schedule_history(id))
    }
    pub fn delete_schedule(&self, id: &str) -> Result<bool> {
        self.shared.access(|db| db.delete_schedule(id))
    }
}
