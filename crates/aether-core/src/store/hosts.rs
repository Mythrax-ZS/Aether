//! Persistent store of saved hosts and groups.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;

use super::json_file::JsonFile;
use crate::error::{CoreError, Result};
use crate::ids::{GroupId, HostId};
use crate::model::{Group, Host, JumpRef};
use crate::util::now_ms;

pub const HOSTS_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct HostsData {
    pub version: u32,
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
}

/// What happens to the hosts inside a group that is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum GroupDeleteMode {
    /// Hosts and subgroups move up to the deleted group's parent.
    MoveContents,
    /// Hosts inside the group (and its subgroups) are deleted too.
    DeleteContents,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportSummary {
    pub added: u32,
    pub skipped: u32,
    /// Jump hosts that could not be linked to an imported or saved host and were left out. Each such host would
    /// otherwise connect directly, around its bastion.
    pub unlinked_jumps: u32,
}

pub struct HostStore {
    file: JsonFile,
    data: RwLock<HostsData>,
    changed: broadcast::Sender<()>,
}

impl HostStore {
    /// Opens (or creates) the store. The second value reports a recovered/corrupt file.
    pub fn open(path: impl Into<PathBuf>) -> (Self, Option<CoreError>) {
        let file = JsonFile::new(path);
        let loaded = file.load_or_default::<HostsData>();
        let mut data = loaded.value;
        data.version = HOSTS_VERSION;
        let (changed, _) = broadcast::channel(16);
        (Self { file, data: RwLock::new(data), changed }, loaded.recovery)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn snapshot(&self) -> HostsData {
        self.data.read().clone()
    }

    pub fn get(&self, id: HostId) -> Option<Host> {
        self.data.read().hosts.iter().find(|h| h.id == id).cloned()
    }

    pub fn group(&self, id: GroupId) -> Option<Group> {
        self.data.read().groups.iter().find(|g| g.id == id).cloned()
    }

    /// Finds a host by its exact connection target.
    pub fn find_by_target(&self, hostname: &str, port: u16, username: &str) -> Option<Host> {
        self.data
            .read()
            .hosts
            .iter()
            .find(|h| h.hostname.eq_ignore_ascii_case(hostname) && h.port == port && h.username == username)
            .cloned()
    }

    fn notify(&self) {
        let _ = self.changed.send(());
    }

    /// Applies `change` to a copy of the data and keeps the result only once it has reached the disk. If saving fails
    /// the list in memory does not run ahead of the file — it would otherwise show hosts that are gone after a restart.
    fn transact<T>(&self, notify: bool, change: impl FnOnce(&mut HostsData) -> Result<T>) -> Result<T> {
        let mut data = self.data.write();
        let mut next = data.clone();
        let out = change(&mut next)?;
        if next == *data {
            return Ok(out); // nothing changed: nothing to write, nobody to tell
        }
        self.file.save(&next)?;
        *data = next;
        drop(data);
        if notify {
            self.notify();
        }
        Ok(out)
    }

    // ── hosts ────────────────────────────────────────────────────────────────

    /// Inserts or updates a host. Statistics (`created_at`, `connect_count`,
    /// `last_connected_at`) are owned by the store and preserved on update.
    pub fn upsert(&self, mut host: Host) -> Result<Host> {
        host.normalize();
        host.validate()?;
        self.transact(true, move |data| {
            if jump_cycle(&data.hosts, &host) {
                return Err(CoreError::invalid("jump hosts", "this chain would loop back to the host itself"));
            }
            if let Some(gid) = host.group_id
                && !data.groups.iter().any(|g| g.id == gid)
            {
                host.group_id = None;
            }
            let now = now_ms();
            host.updated_at = now;
            match data.hosts.iter().position(|h| h.id == host.id) {
                Some(i) => {
                    let existing = &data.hosts[i];
                    host.created_at = existing.created_at;
                    host.last_connected_at = existing.last_connected_at;
                    host.connect_count = existing.connect_count;
                    data.hosts[i] = host.clone();
                }
                None => {
                    host.created_at = now;
                    host.connect_count = 0;
                    host.last_connected_at = None;
                    if host.sort == 0 {
                        host.sort = data
                            .hosts
                            .iter()
                            .filter(|h| h.group_id == host.group_id)
                            .map(|h| h.sort)
                            .max()
                            .map_or(0, |m| m + 1);
                    }
                    data.hosts.push(host.clone());
                }
            }
            Ok(host)
        })
    }

    /// Deletes hosts and removes them from every other host's jump chain.
    pub fn delete(&self, ids: &[HostId]) -> Result<()> {
        let remove: HashSet<HostId> = ids.iter().copied().collect();
        self.transact(true, |data| {
            let before = data.hosts.len();
            data.hosts.retain(|h| !remove.contains(&h.id));
            if data.hosts.len() == before {
                return Err(CoreError::not_found("host"));
            }
            for h in &mut data.hosts {
                h.jump.retain(|j| !matches!(j, JumpRef::Host { host_id } if remove.contains(host_id)));
            }
            Ok(())
        })
    }

    pub fn duplicate(&self, id: HostId) -> Result<Host> {
        let mut copy = self.get(id).ok_or_else(|| CoreError::not_found("host"))?;
        copy.id = HostId::new();
        copy.label = format!("{} copy", copy.display_name());
        copy.favorite = false;
        copy.sort = 0;
        for f in &mut copy.forwards {
            f.id = uuid::Uuid::new_v4().to_string();
        }
        self.upsert(copy)
    }

    /// Records a successful connection (powers "recent" lists).
    pub fn touch_connected(&self, id: HostId) -> Result<()> {
        self.transact(true, |data| {
            let host = data.hosts.iter_mut().find(|h| h.id == id).ok_or_else(|| CoreError::not_found("host"))?;
            host.last_connected_at = Some(now_ms());
            host.connect_count = host.connect_count.saturating_add(1);
            Ok(())
        })
    }

    pub fn set_favorite(&self, id: HostId, favorite: bool) -> Result<()> {
        self.transact(true, |data| {
            let host = data.hosts.iter_mut().find(|h| h.id == id).ok_or_else(|| CoreError::not_found("host"))?;
            host.favorite = favorite;
            host.updated_at = now_ms();
            Ok(())
        })
    }

    /// Moves hosts into a group (or to the top level with `None`) and appends them in order.
    pub fn move_hosts(&self, ids: &[HostId], group_id: Option<GroupId>) -> Result<()> {
        self.transact(true, |data| {
            if let Some(gid) = group_id
                && !data.groups.iter().any(|g| g.id == gid)
            {
                return Err(CoreError::not_found("group"));
            }
            let mut next = data
                .hosts
                .iter()
                .filter(|h| h.group_id == group_id && !ids.contains(&h.id))
                .map(|h| h.sort)
                .max()
                .map_or(0, |m| m + 1);
            for id in ids {
                if let Some(h) = data.hosts.iter_mut().find(|h| h.id == *id) {
                    h.group_id = group_id;
                    h.sort = next;
                    next += 1;
                }
            }
            Ok(())
        })
    }

    /// Applies a manual ordering to the given hosts (in the order supplied).
    pub fn reorder_hosts(&self, ordered: &[HostId]) -> Result<()> {
        self.transact(true, |data| {
            for (i, id) in ordered.iter().enumerate() {
                if let Some(h) = data.hosts.iter_mut().find(|h| h.id == *id) {
                    h.sort = i as i32;
                }
            }
            Ok(())
        })
    }

    /// Adds hosts that aren't already saved: the same alias imported before, or the same user@host:port *reached the
    /// same way* (the same jump hosts and proxy command). Two aliases for one machine that differ in how it is reached
    /// — `prod` through a bastion, `prod-direct` without — are two hosts: merging them would send a link meant to go
    /// through the bastion around it.
    ///
    /// Jump links between hosts of the same batch hold whatever order they come in, and a link to a host that was
    /// skipped as a duplicate is pointed at the saved host it duplicates. A link that can't be kept is dropped and
    /// counted in [`ImportSummary::unlinked_jumps`] — the host would otherwise quietly connect without its bastion.
    pub fn import(&self, incoming: Vec<Host>, group_id: Option<GroupId>) -> Result<ImportSummary> {
        let mut summary = ImportSummary::default();
        self.transact(true, |data| {
            let group_id = group_id.filter(|g| data.groups.iter().any(|x| x.id == *g));
            // How a host is reached, written so that hosts of the saved list and of this batch can be compared.
            let endpoint = |h: &Host| format!("{}@{}:{}", h.username, h.hostname.to_ascii_lowercase(), h.port);
            let endpoints: HashMap<HostId, String> =
                data.hosts.iter().chain(incoming.iter()).map(|h| (h.id, endpoint(h))).collect();
            let reach = |h: &Host| -> (Vec<String>, String) {
                let jumps = h
                    .jump
                    .iter()
                    .map(|j| match j {
                        JumpRef::Host { host_id } => {
                            endpoints.get(host_id).cloned().unwrap_or_else(|| host_id.to_string())
                        }
                        JumpRef::Adhoc { spec } => spec.trim().to_ascii_lowercase(),
                    })
                    .collect();
                (jumps, h.options.proxy_command.trim().to_string())
            };
            let same_machine = |a: &Host, b: &Host| {
                (a.origin == b.origin && !matches!(b.origin, crate::model::HostOrigin::Manual))
                    || (a.hostname.eq_ignore_ascii_case(&b.hostname)
                        && a.port == b.port
                        && a.username == b.username
                        && reach(a) == reach(b))
            };

            // Pass 1: what is new, and where each incoming host ended up (itself, or the saved host it duplicates).
            let mut landed: HashMap<HostId, HostId> = HashMap::new();
            let mut fresh: Vec<Host> = Vec::new();
            for mut host in incoming {
                host.normalize();
                if host.validate().is_err() {
                    summary.skipped += 1;
                    continue;
                }
                match data.hosts.iter().chain(fresh.iter()).find(|h| same_machine(h, &host)) {
                    Some(existing) => {
                        landed.insert(host.id, existing.id);
                        summary.skipped += 1;
                    }
                    None => {
                        landed.insert(host.id, host.id);
                        fresh.push(host);
                    }
                }
            }
            // Pass 2: place them and settle their jump links.
            let known: HashSet<HostId> = data.hosts.iter().chain(fresh.iter()).map(|h| h.id).collect();
            for mut host in fresh {
                host.group_id = group_id;
                host.sort = data
                    .hosts
                    .iter()
                    .filter(|h| h.group_id == host.group_id)
                    .map(|h| h.sort)
                    .max()
                    .map_or(0, |m| m + 1);
                host.created_at = now_ms();
                host.updated_at = host.created_at;
                let own_id = host.id;
                host.jump = std::mem::take(&mut host.jump)
                    .into_iter()
                    .filter_map(|j| match j {
                        JumpRef::Host { host_id } => {
                            let target = landed.get(&host_id).copied().unwrap_or(host_id);
                            if target != own_id && known.contains(&target) {
                                Some(JumpRef::Host { host_id: target })
                            } else {
                                summary.unlinked_jumps += 1;
                                None
                            }
                        }
                        adhoc => Some(adhoc),
                    })
                    .collect();
                data.hosts.push(host);
                summary.added += 1;
            }
            Ok(())
        })?;
        Ok(summary)
    }

    // ── groups ───────────────────────────────────────────────────────────────

    pub fn upsert_group(&self, mut group: Group) -> Result<Group> {
        group.name = group.name.trim().to_string();
        if group.name.is_empty() {
            return Err(CoreError::invalid("group name", "give the group a name"));
        }
        self.transact(true, move |data| {
            if let Some(pid) = group.parent_id {
                if pid == group.id || !data.groups.iter().any(|g| g.id == pid) {
                    return Err(CoreError::invalid("parent group", "pick a different parent"));
                }
                // A group cannot be moved inside one of its own descendants.
                let mut cursor = Some(pid);
                let mut guard = 0;
                while let Some(c) = cursor {
                    if c == group.id {
                        return Err(CoreError::invalid("parent group", "a group cannot be moved inside itself"));
                    }
                    cursor = data.groups.iter().find(|g| g.id == c).and_then(|g| g.parent_id);
                    guard += 1;
                    if guard > 64 {
                        break;
                    }
                }
            }
            match data.groups.iter().position(|g| g.id == group.id) {
                Some(i) => data.groups[i] = group.clone(),
                None => {
                    group.sort = data
                        .groups
                        .iter()
                        .filter(|g| g.parent_id == group.parent_id)
                        .map(|g| g.sort)
                        .max()
                        .map_or(0, |m| m + 1);
                    data.groups.push(group.clone());
                }
            }
            Ok(group)
        })
    }

    pub fn set_group_collapsed(&self, id: GroupId, collapsed: bool) -> Result<()> {
        // Collapsing is UI state; no need to broadcast a full refresh.
        self.transact(false, |data| {
            let g = data.groups.iter_mut().find(|g| g.id == id).ok_or_else(|| CoreError::not_found("group"))?;
            g.collapsed = collapsed;
            Ok(())
        })
    }

    pub fn delete_group(&self, id: GroupId, mode: GroupDeleteMode) -> Result<()> {
        self.transact(true, |data| {
            let parent =
                data.groups.iter().find(|g| g.id == id).ok_or_else(|| CoreError::not_found("group"))?.parent_id;

            match mode {
                GroupDeleteMode::MoveContents => {
                    for g in data.groups.iter_mut().filter(|g| g.parent_id == Some(id)) {
                        g.parent_id = parent;
                    }
                    for h in data.hosts.iter_mut().filter(|h| h.group_id == Some(id)) {
                        h.group_id = parent;
                    }
                    data.groups.retain(|g| g.id != id);
                }
                GroupDeleteMode::DeleteContents => {
                    let mut doomed: HashSet<GroupId> = HashSet::from([id]);
                    loop {
                        let before = doomed.len();
                        let more: Vec<GroupId> = data
                            .groups
                            .iter()
                            .filter(|g| g.parent_id.is_some_and(|p| doomed.contains(&p)))
                            .map(|g| g.id)
                            .collect();
                        doomed.extend(more);
                        if doomed.len() == before {
                            break;
                        }
                    }
                    let removed_hosts: HashSet<HostId> = data
                        .hosts
                        .iter()
                        .filter(|h| h.group_id.is_some_and(|g| doomed.contains(&g)))
                        .map(|h| h.id)
                        .collect();
                    data.groups.retain(|g| !doomed.contains(&g.id));
                    data.hosts.retain(|h| !removed_hosts.contains(&h.id));
                    for h in &mut data.hosts {
                        h.jump.retain(|j| !matches!(j, JumpRef::Host { host_id } if removed_hosts.contains(host_id)));
                    }
                }
            }
            Ok(())
        })
    }
}

/// Would giving `candidate` its jump chain create a loop?
fn jump_cycle(hosts: &[Host], candidate: &Host) -> bool {
    let mut stack: Vec<HostId> = candidate
        .jump
        .iter()
        .filter_map(|j| match j {
            JumpRef::Host { host_id } => Some(*host_id),
            JumpRef::Adhoc { .. } => None,
        })
        .collect();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == candidate.id {
            return true;
        }
        if !seen.insert(id) {
            continue;
        }
        if let Some(h) = hosts.iter().find(|h| h.id == id) {
            for j in &h.jump {
                if let JumpRef::Host { host_id } = j {
                    stack.push(*host_id);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, HostStore) {
        let dir = tempfile::tempdir().unwrap();
        let (store, note) = HostStore::open(dir.path().join("hosts.json"));
        assert!(note.is_none());
        (dir, store)
    }

    #[test]
    fn upsert_persists_and_reloads() {
        let (dir, store) = store();
        let mut h = Host::new("example.com");
        h.username = "me".into();
        let saved = store.upsert(h).unwrap();
        assert_eq!(saved.label, "example.com");

        let (reopened, _) = HostStore::open(dir.path().join("hosts.json"));
        assert_eq!(reopened.snapshot().hosts.len(), 1);
        assert_eq!(reopened.get(saved.id).unwrap().username, "me");
    }

    #[test]
    fn update_preserves_store_owned_statistics() {
        let (_d, store) = store();
        let h = store.upsert(Host::new("a.example")).unwrap();
        store.touch_connected(h.id).unwrap();
        store.touch_connected(h.id).unwrap();
        let mut edited = h.clone();
        edited.label = "renamed".into();
        edited.connect_count = 0; // a stale client copy must not clobber stats
        let saved = store.upsert(edited).unwrap();
        assert_eq!(saved.connect_count, 2);
        assert!(saved.last_connected_at.is_some());
    }

    #[test]
    fn delete_cleans_jump_references() {
        let (_d, store) = store();
        let bastion = store.upsert(Host::new("bastion")).unwrap();
        let mut target = Host::new("internal");
        target.jump = vec![JumpRef::Host { host_id: bastion.id }];
        let target = store.upsert(target).unwrap();

        store.delete(&[bastion.id]).unwrap();
        assert!(store.get(target.id).unwrap().jump.is_empty());
        assert!(store.delete(&[bastion.id]).is_err(), "second delete reports not-found");
    }

    #[test]
    fn jump_cycles_are_rejected() {
        let (_d, store) = store();
        let a = store.upsert(Host::new("a")).unwrap();
        let mut b = Host::new("b");
        b.jump = vec![JumpRef::Host { host_id: a.id }];
        let b = store.upsert(b).unwrap();

        // a -> b -> a would loop.
        let mut a2 = a.clone();
        a2.jump = vec![JumpRef::Host { host_id: b.id }];
        assert!(store.upsert(a2).is_err());
    }

    #[test]
    fn duplicate_makes_independent_copy() {
        let (_d, store) = store();
        let mut h = Host::new("web");
        h.favorite = true;
        h.forwards = vec![crate::model::ForwardRule { bind_port: 8080, dest_port: 80, ..Default::default() }];
        let h = store.upsert(h).unwrap();
        let copy = store.duplicate(h.id).unwrap();
        assert_ne!(copy.id, h.id);
        assert_eq!(copy.label, "web copy");
        assert!(!copy.favorite);
        assert_ne!(copy.forwards[0].id, h.forwards[0].id);
    }

    #[test]
    fn group_lifecycle() {
        let (_d, store) = store();
        let prod = store.upsert_group(Group::named("Production")).unwrap();
        let eu = store.upsert_group(Group { parent_id: Some(prod.id), ..Group::named("EU") }).unwrap();
        let mut h = Host::new("eu-web");
        h.group_id = Some(eu.id);
        let h = store.upsert(h).unwrap();

        // A group can't become its own descendant's child.
        let mut prod2 = prod.clone();
        prod2.parent_id = Some(eu.id);
        assert!(store.upsert_group(prod2).is_err());

        store.delete_group(eu.id, GroupDeleteMode::MoveContents).unwrap();
        assert_eq!(store.get(h.id).unwrap().group_id, Some(prod.id));

        store.delete_group(prod.id, GroupDeleteMode::DeleteContents).unwrap();
        assert!(store.get(h.id).is_none());
        assert!(store.snapshot().groups.is_empty());
    }

    #[test]
    fn import_skips_duplicates() {
        let (_d, store) = store();
        let mut existing = Host::new("dup.example");
        existing.username = "u".into();
        store.upsert(existing).unwrap();

        let mut dup = Host::new("dup.example");
        dup.username = "u".into();
        let fresh = Host::new("fresh.example");
        let summary = store.import(vec![dup, fresh], None).unwrap();
        assert_eq!(summary, ImportSummary { added: 1, skipped: 1, unlinked_jumps: 0 });
        assert_eq!(store.snapshot().hosts.len(), 2);
    }

    fn via(host: &mut Host, jump: &Host) {
        host.jump.push(JumpRef::Host { host_id: jump.id });
    }

    /// `prod` comes before the `bastion` it jumps through. The link must survive that order.
    #[test]
    fn imported_hosts_keep_their_jump_links_in_any_order() {
        let (_d, store) = store();
        let bastion = Host::new("bastion.example");
        let mut prod = Host::new("prod.example");
        via(&mut prod, &bastion);
        let (bastion_id, prod_id) = (bastion.id, prod.id);

        let summary = store.import(vec![prod, bastion], None).unwrap();
        assert_eq!(summary, ImportSummary { added: 2, skipped: 0, unlinked_jumps: 0 });
        assert_eq!(store.get(prod_id).unwrap().jump, [JumpRef::Host { host_id: bastion_id }]);
    }

    /// The bastion is already saved (and is skipped as a duplicate): the new host links to the saved one.
    #[test]
    fn a_link_to_a_skipped_duplicate_points_at_the_saved_host() {
        let (_d, store) = store();
        let saved = store.upsert(Host::new("bastion.example")).unwrap();
        let bastion_again = Host::new("bastion.example");
        let mut prod = Host::new("prod.example");
        via(&mut prod, &bastion_again);
        let prod_id = prod.id;

        let summary = store.import(vec![bastion_again, prod], None).unwrap();
        assert_eq!(summary, ImportSummary { added: 1, skipped: 1, unlinked_jumps: 0 });
        assert_eq!(store.get(prod_id).unwrap().jump, [JumpRef::Host { host_id: saved.id }]);
    }

    /// A link that goes nowhere is dropped *and counted*; the ad-hoc ones are untouched.
    #[test]
    fn links_that_cannot_be_kept_are_counted_not_silently_lost() {
        let (_d, store) = store();
        let stranger = Host::new("never-imported.example");
        let mut prod = Host::new("prod.example");
        via(&mut prod, &stranger);
        prod.jump.push(JumpRef::Adhoc { spec: "ops@jump.example:2200".into() });
        let prod_id = prod.id;

        let summary = store.import(vec![prod], None).unwrap();
        assert_eq!(summary, ImportSummary { added: 1, skipped: 0, unlinked_jumps: 1 });
        assert_eq!(store.get(prod_id).unwrap().jump, [JumpRef::Adhoc { spec: "ops@jump.example:2200".into() }]);
    }

    /// Two aliases for one machine that are reached differently stay two hosts. Merged, the one that goes through a
    /// bastion would become the one that goes around it.
    #[test]
    fn aliases_reached_differently_are_not_duplicates_of_each_other() {
        let (_d, store) = store();
        let edge = Host::new("edge.example");
        let mut direct = Host::new("prod.example");
        direct.label = "prod-direct".into();
        let mut guarded = Host::new("prod.example");
        guarded.label = "prod".into();
        via(&mut guarded, &edge);
        let (direct_id, guarded_id, edge_id) = (direct.id, guarded.id, edge.id);

        let summary = store.import(vec![direct, guarded, edge], None).unwrap();
        assert_eq!(summary, ImportSummary { added: 3, skipped: 0, unlinked_jumps: 0 });
        assert!(store.get(direct_id).unwrap().jump.is_empty());
        assert_eq!(store.get(guarded_id).unwrap().jump, [JumpRef::Host { host_id: edge_id }]);

        // The same machine reached the same way is still the same host, whichever name it came under.
        let mut again = Host::new("prod.example");
        again.label = "prod-again".into();
        via(&mut again, &Host { id: edge_id, ..Host::new("edge.example") });
        assert_eq!(store.import(vec![again], None).unwrap(), ImportSummary { added: 0, skipped: 1, unlinked_jumps: 0 });
        // And a proxy command is part of how it is reached.
        let mut proxied = Host::new("prod.example");
        proxied.options.proxy_command = "ssh -W %h:%p other".into();
        assert_eq!(store.import(vec![proxied], None).unwrap().added, 1);
    }

    #[test]
    fn importing_nothing_new_writes_nothing() {
        let (dir, store) = store();
        store.upsert(Host::new("a.example")).unwrap();
        let path = dir.path().join("hosts.json");
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let summary = store.import(vec![Host::new("a.example")], None).unwrap();
        assert_eq!(summary.added, 0);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "the file was left alone");
    }

    /// If the disk refuses the write, memory must not have moved on without it.
    #[test]
    fn a_failed_save_leaves_the_list_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        let (store, _) = HostStore::open(&path);
        store.upsert(Host::new("kept.example")).unwrap();

        // Make saving impossible: the destination becomes a directory.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.upsert(Host::new("lost.example")).is_err());
        let names: Vec<String> = store.snapshot().hosts.into_iter().map(|h| h.hostname).collect();
        assert_eq!(names, ["kept.example"], "the host that could not be saved is not on screen either");
    }

    #[test]
    fn move_and_reorder() {
        let (_d, store) = store();
        let g = store.upsert_group(Group::named("G")).unwrap();
        let a = store.upsert(Host::new("a")).unwrap();
        let b = store.upsert(Host::new("b")).unwrap();
        store.move_hosts(&[b.id, a.id], Some(g.id)).unwrap();
        let snap = store.snapshot();
        let sa = snap.hosts.iter().find(|h| h.id == a.id).unwrap();
        let sb = snap.hosts.iter().find(|h| h.id == b.id).unwrap();
        assert_eq!((sa.group_id, sb.group_id), (Some(g.id), Some(g.id)));
        assert!(sb.sort < sa.sort);
        assert!(store.move_hosts(&[a.id], Some(GroupId::new())).is_err());
    }
}
