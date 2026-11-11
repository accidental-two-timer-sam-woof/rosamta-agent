//! `rsmt recent` and `rsmt include`.

use std::process::ExitCode;

use rosamta_core::apply::Context;
use rosamta_core::hooks::FileChange;
use rosamta_core::include::{self, Blocked, Item, Unlisted};
use rosamta_core::scope::{self, Widening};
use rosamta_core::{Error, Result, adopt, time};
use serde::Serialize;

use crate::commands::confirm;
use crate::print_json;
use crate::sync::after_commit;

pub fn recent(json: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let items = include::recent(&ctx)?;
    if json {
        print_json(&items);
        return Ok(ExitCode::SUCCESS);
    }
    if items.is_empty() {
        println!("Everything in the repo loads on this host");
        return Ok(ExitCode::SUCCESS);
    }
    let added: Vec<String> = items
        .iter()
        .map(|item| {
            item.added
                .map_or_else(|| "not committed".into(), time::date)
        })
        .collect();
    let width = items.iter().map(|item| item.name.len()).max().unwrap_or(0);
    let date_width = added.iter().map(String::len).max().unwrap_or(0);
    for (item, added) in items.iter().zip(&added) {
        let mut line = format!(
            "{:<width$}  {added:<date_width$}  {}",
            item.name,
            scope_of(item)
        );
        if let Some(blocked) = &item.blocked {
            line.push_str(&format!(" ({})", why(blocked)));
        }
        println!("{}", line.trim_end());
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct Included<'a> {
    included: &'a [Item],
    skipped: &'a [Item],
}

pub fn include(json: bool, names: &[String], all: bool, yes: bool) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    let recent = include::recent(&ctx)?;
    let chosen = if all {
        recent
    } else {
        names
            .iter()
            .map(|name| find(&ctx, &recent, name))
            .collect::<Result<Vec<_>>>()?
    };
    let (skipped, chosen): (Vec<Item>, Vec<Item>) =
        chosen.into_iter().partition(|item| item.blocked.is_some());
    if !json {
        for item in &skipped {
            let blocked = item.blocked.as_ref().expect("partitioned on blocked");
            println!("Skipped {}: {}", item.name, why(blocked));
        }
    }
    if chosen.is_empty() {
        if json {
            print_json(&Included {
                included: &[],
                skipped: &skipped,
            });
        } else {
            println!("Nothing to include");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let changes: Vec<FileChange> = include::plan(&ctx, &chosen)?;
    if !json && !yes {
        for item in &chosen {
            println!("{} → {}", item.name, adds(&item.widening));
        }
    }
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    include::include(&ctx, &chosen, &changes)?;
    if json {
        print_json(&Included {
            included: &chosen,
            skipped: &skipped,
        });
    } else {
        let names: Vec<&str> = chosen.iter().map(|item| item.name.as_str()).collect();
        println!("Included {}", names.join(", "));
    }
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

/// The listed item `name` stands for; a usage error saying why it is not
/// listed otherwise.
fn find(ctx: &Context, recent: &[Item], name: &str) -> Result<Item> {
    let shown = include::shown(ctx, name);
    if let Some(item) = recent.iter().find(|item| item.name == shown) {
        return Ok(item.clone());
    }
    Err(Error::Usage(match include::unlisted(ctx, &shown)? {
        Unlisted::Loaded => format!("{shown} already loads on this host"),
        Unlisted::Disabled => format!("{shown} is turned off"),
        Unlisted::Missing => format!("{shown} is not in the repo"),
    }))
}

/// `mbp · macos · zsh`: the hosts, OS, architectures, tags and shells it is
/// limited to.
fn scope_of(item: &Item) -> String {
    let when = &item.when;
    let groups = [
        when.hosts.clone(),
        when.os.iter().map(scope::name).collect(),
        when.arch.iter().map(scope::name).collect(),
        when.tags.iter().map(|tag| format!("tag {tag}")).collect(),
        item.shells.iter().map(ToString::to_string).collect(),
    ];
    groups
        .iter()
        .filter(|group| !group.is_empty())
        .map(|group| group.join(", "))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `hosts + box, os + linux`.
fn adds(widening: &Widening) -> String {
    let mut parts = Vec::new();
    if let Some(host) = &widening.host {
        parts.push(format!("hosts + {host}"));
    }
    if let Some(os) = widening.os {
        parts.push(format!("os + {}", scope::name(os)));
    }
    if let Some(arch) = widening.arch {
        parts.push(format!("arch + {}", scope::name(arch)));
    }
    if !widening.shells.is_empty() {
        let shells: Vec<String> = widening.shells.iter().map(ToString::to_string).collect();
        parts.push(format!("shells + {}", shells.join(", ")));
    }
    parts.join(", ")
}

fn why(blocked: &Blocked) -> String {
    match blocked {
        Blocked::Shells(shells) => {
            let shells: Vec<String> = shells.iter().map(ToString::to_string).collect();
            format!("only for {}", shells.join(", "))
        }
        Blocked::Tags(tags) => format!("needs tag {}", tags.join(" or ")),
    }
}
