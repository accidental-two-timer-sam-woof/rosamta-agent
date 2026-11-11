//! Scripts in the review queue: the ones this host has yet to run, kept up
//! to date whenever applying or reviewing looks at them.

use serde::{Deserialize, Serialize};

use super::Run;
use super::apply::{Pending, Why, key, pending, remember};
use super::run::{self, Outcome};
use crate::adopt::store::{QueueItem, Store};
use crate::adopt::{self, update_notice};
use crate::apply::Context;
use crate::error::{Error, Result};
use crate::time::now;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptItem {
    /// The script's source in the repo.
    pub source: String,
    pub run: Run,
    pub why: Why,
    /// The script as listed: accepting runs this version only.
    pub hash: String,
}

fn item(pending: &Pending) -> ScriptItem {
    ScriptItem {
        source: key(&pending.script),
        run: pending.script.run,
        why: pending.why,
        hash: pending.hash.clone(),
    }
}

/// Make the queue's script items the scripts this host has yet to run. An
/// item keeps its ID while its source is the same.
pub fn sync_queue(ctx: &Context) -> Result<()> {
    let host = adopt::host_config(ctx)?;
    let wanted: Vec<ScriptItem> = pending(ctx, &host)?.iter().map(item).collect();
    let store = Store::new(ctx.paths.state());
    let mut queue = store.queue()?;
    let (old, mut items): (Vec<QueueItem>, Vec<QueueItem>) = queue
        .items
        .into_iter()
        .partition(|item| item.script.is_some());
    let same = wanted.len() == old.len()
        && wanted
            .iter()
            .all(|item| old.iter().any(|old| old.script.as_ref() == Some(item)));
    if same {
        queue.items = items;
        queue.items.extend(old);
        queue.items.sort_by_key(|item| item.id);
        return Ok(());
    }
    queue.next_id = queue.next_id.max(1);
    for script in wanted {
        let kept = old
            .iter()
            .find(|old| {
                old.script
                    .as_ref()
                    .is_some_and(|old| old.source == script.source)
            })
            .map(|old| (old.id, old.detected_at));
        let (id, detected_at) = kept.unwrap_or_else(|| {
            queue.next_id += 1;
            (queue.next_id - 1, now())
        });
        items.push(QueueItem {
            id,
            file: ctx.paths.repo().join(&script.source),
            detected_at,
            script: Some(script),
            ..Default::default()
        });
    }
    items.sort_by_key(|item| item.id);
    queue.items = items;
    store.save_queue(&queue)?;
    update_notice(&store, &host, queue.items.len())
}

/// The pending script `listed` stands for, as long as it is still the
/// version that was listed.
fn current(ctx: &Context, id: u32, listed: &ScriptItem) -> Result<Pending> {
    let host = adopt::host_config(ctx)?;
    let found = pending(ctx, &host)?
        .into_iter()
        .find(|pending| key(&pending.script) == listed.source);
    match found {
        Some(pending) if pending.hash == listed.hash => Ok(pending),
        _ => {
            sync_queue(ctx)?;
            Err(Error::Usage(format!(
                "review item {id} changed since it was listed; run `rsmt review`"
            )))
        }
    }
}

/// The script item `id`, if it is one.
pub fn item_of(ctx: &Context, id: u32) -> Result<Option<ScriptItem>> {
    Ok(Store::new(ctx.paths.state())
        .queue()?
        .items
        .into_iter()
        .find(|item| item.id == id)
        .and_then(|item| item.script))
}

/// `rsmt review accept` on a script: run it, in the terminal when there
/// is one. The item goes once the script succeeds. The caller holds the
/// adopt lock.
pub fn accept(ctx: &Context, id: u32, listed: &ScriptItem) -> Result<Outcome> {
    let pending = current(ctx, id, listed)?;
    let host = adopt::host_config(ctx)?;
    let outcome = run::run(ctx, &host.id, &pending.script, ctx.interactive)?;
    remember(ctx, &pending, outcome.ok)?;
    sync_queue(ctx)?;
    Ok(outcome)
}

/// `rsmt review ignore` on a script: count it as run without running it.
pub(crate) fn ignore(ctx: &Context, id: u32, listed: &ScriptItem) -> Result<()> {
    let pending = current(ctx, id, listed)?;
    remember(ctx, &pending, true)?;
    sync_queue(ctx)
}
