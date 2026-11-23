//! Live connections: paired machines and signed-in browsers, and who watches which
//! terminal. A terminal streams only while at least one browser watches it.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

pub type Tx = UnboundedSender<Outgoing>;

/// What a connection task sends to its socket.
#[derive(Debug, Clone)]
pub enum Outgoing {
    Text(String),
    /// Close the socket (the machine was removed, or the browser signed out).
    Close,
}

struct MachineConn {
    conn: u64,
    tx: Tx,
}

struct BrowserConn {
    tx: Tx,
    /// Web session this connection belongs to, so signing out closes it.
    web_session: i64,
    watching: HashSet<String>,
}

#[derive(Default)]
struct Inner {
    machines: HashMap<i64, MachineConn>,
    browsers: HashMap<u64, BrowserConn>,
    /// session id → (its machine, the browsers watching it)
    watchers: HashMap<String, (i64, HashSet<u64>)>,
    /// Bumped on each (dis)connect, so a delayed "offline" check can tell if it's stale.
    machine_generation: HashMap<i64, u64>,
}

#[derive(Default)]
pub struct Hub {
    inner: Mutex<Inner>,
    next: AtomicU64,
    /// Screen sessions (`screen.rs`).
    pub screen: crate::screen::ScreenHub,
}

fn send(tx: &Tx, v: &Value) {
    let _ = tx.send(Outgoing::Text(v.to_string()));
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    // ---- machines ----------------------------------------------------------------------

    /// Registers a machine's connection (replacing an older one). Returns the generation.
    pub fn machine_connected(&self, machine: i64, conn: u64, tx: Tx) -> u64 {
        let mut g = self.lock();
        if let Some(old) = g.machines.insert(
            machine,
            MachineConn {
                conn,
                tx: tx.clone(),
            },
        ) {
            let _ = old.tx.send(Outgoing::Close);
        }
        let generation = g.machine_generation.entry(machine).or_default();
        *generation += 1;
        let generation = *generation;
        // Terminals someone is already watching start streaming again.
        let watched: Vec<String> = g
            .watchers
            .iter()
            .filter(|(_, (m, set))| *m == machine && !set.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        for id in watched {
            send(&tx, &json!({ "type": "watch", "id": id, "on": true }));
        }
        generation
    }

    /// Unregisters it if `conn` is still the current connection. Returns the new
    /// generation when it was, so the caller can mark the machine offline.
    pub fn machine_disconnected(&self, machine: i64, conn: u64) -> Option<u64> {
        let mut g = self.lock();
        if g.machines.get(&machine).is_some_and(|m| m.conn == conn) {
            g.machines.remove(&machine);
            let generation = g.machine_generation.entry(machine).or_default();
            *generation += 1;
            let generation = *generation;
            let ids: Vec<(String, Vec<u64>)> = g
                .watchers
                .iter()
                .filter(|(_, (m, _))| *m == machine)
                .map(|(id, (_, set))| (id.clone(), set.iter().copied().collect()))
                .collect();
            for (id, browsers) in ids {
                for b in browsers {
                    if let Some(bc) = g.browsers.get(&b) {
                        send(&bc.tx, &json!({ "type": "watch", "id": id, "live": false }));
                    }
                }
            }
            Some(generation)
        } else {
            None
        }
    }

    pub fn machine_generation(&self, machine: i64) -> u64 {
        self.lock()
            .machine_generation
            .get(&machine)
            .copied()
            .unwrap_or(0)
    }

    pub fn machine_online(&self, machine: i64) -> bool {
        self.lock().machines.contains_key(&machine)
    }

    /// Sends a message to a machine. False when it isn't connected.
    pub fn to_machine(&self, machine: i64, v: &Value) -> bool {
        match self.lock().machines.get(&machine) {
            Some(m) => m.tx.send(Outgoing::Text(v.to_string())).is_ok(),
            None => false,
        }
    }

    /// The machine was removed: tell it, then close its socket.
    pub fn revoke_machine(&self, machine: i64) {
        self.screen.close_machine(machine);
        let mut g = self.lock();
        if let Some(m) = g.machines.remove(&machine) {
            send(
                &m.tx,
                &json!({ "type": "bye", "reason": "This computer was removed from the server." }),
            );
            let _ = m.tx.send(Outgoing::Close);
        }
        g.watchers.retain(|_, (m, _)| *m != machine);
    }

    // ---- browsers ----------------------------------------------------------------------

    pub fn browser_connected(&self, conn: u64, web_session: i64, tx: Tx) {
        self.lock().browsers.insert(
            conn,
            BrowserConn {
                tx,
                web_session,
                watching: HashSet::new(),
            },
        );
    }

    pub fn browser_disconnected(&self, conn: u64) {
        let mut g = self.lock();
        let Some(b) = g.browsers.remove(&conn) else {
            return;
        };
        for id in b.watching {
            Self::unwatch_locked(&mut g, conn, &id);
        }
    }

    /// Closes every browser connection of a web session (it signed out or was revoked).
    pub fn close_web_session(&self, web_session: i64) {
        self.screen.close_web_session(web_session);
        let g = self.lock();
        for b in g.browsers.values().filter(|b| b.web_session == web_session) {
            let _ = b.tx.send(Outgoing::Close);
        }
    }

    pub fn broadcast(&self, v: &Value) {
        let text = v.to_string();
        for b in self.lock().browsers.values() {
            let _ = b.tx.send(Outgoing::Text(text.clone()));
        }
    }

    /// A browser opens (`on`) or closes a terminal of `session` on `machine`.
    pub fn watch(&self, conn: u64, session: &str, machine: i64, on: bool) {
        let mut g = self.lock();
        if !g.browsers.contains_key(&conn) {
            return;
        }
        if on {
            let entry = g
                .watchers
                .entry(session.to_string())
                .or_insert((machine, HashSet::new()));
            entry.0 = machine;
            let first = entry.1.is_empty();
            entry.1.insert(conn);
            if let Some(b) = g.browsers.get_mut(&conn) {
                b.watching.insert(session.to_string());
            }
            let online = g.machines.contains_key(&machine);
            if let Some(b) = g.browsers.get(&conn) {
                send(
                    &b.tx,
                    &json!({ "type": "watch", "id": session, "live": online }),
                );
            }
            if online {
                if first {
                    if let Some(m) = g.machines.get(&machine) {
                        send(
                            &m.tx,
                            &json!({ "type": "watch", "id": session, "on": true }),
                        );
                    }
                } else if let Some(m) = g.machines.get(&machine) {
                    // A new viewer needs the screen too: ask for a fresh snapshot.
                    send(
                        &m.tx,
                        &json!({ "type": "watch", "id": session, "on": true }),
                    );
                }
            }
        } else {
            if let Some(b) = g.browsers.get_mut(&conn) {
                b.watching.remove(session);
            }
            Self::unwatch_locked(&mut g, conn, session);
        }
    }

    fn unwatch_locked(g: &mut Inner, conn: u64, session: &str) {
        let Some((machine, set)) = g.watchers.get_mut(session) else {
            return;
        };
        set.remove(&conn);
        if set.is_empty() {
            let machine = *machine;
            g.watchers.remove(session);
            if let Some(m) = g.machines.get(&machine) {
                send(
                    &m.tx,
                    &json!({ "type": "watch", "id": session, "on": false }),
                );
            }
        }
    }

    /// Terminal output from a machine, to everyone watching that session.
    pub fn output(&self, machine: i64, msg: &Value) {
        let Some(id) = msg.get("id").and_then(Value::as_str) else {
            return;
        };
        let g = self.lock();
        let Some((m, set)) = g.watchers.get(id) else {
            return;
        };
        if *m != machine {
            return;
        }
        let text = msg.to_string();
        for conn in set {
            if let Some(b) = g.browsers.get(conn) {
                let _ = b.tx.send(Outgoing::Text(text.clone()));
            }
        }
    }

    /// Keystrokes from a browser that watches the session.
    pub fn input(&self, conn: u64, session: &str, data: &str) -> bool {
        self.relay_from_watcher(
            conn,
            session,
            &json!({ "type": "input", "id": session, "data": data }),
        )
    }

    /// The terminal size a watching browser wants (None: it lets go, the app's size returns).
    pub fn resize(&self, conn: u64, session: &str, size: Option<(u16, u16)>) -> bool {
        let msg = match size {
            Some((cols, rows)) => {
                json!({ "type": "resize", "id": session, "cols": cols, "rows": rows })
            }
            None => json!({ "type": "resize", "id": session, "release": true }),
        };
        self.relay_from_watcher(conn, session, &msg)
    }

    /// Passes a message to the session's machine, only from a browser that watches it.
    fn relay_from_watcher(&self, conn: u64, session: &str, msg: &Value) -> bool {
        let g = self.lock();
        let Some((machine, set)) = g.watchers.get(session) else {
            return false;
        };
        if !set.contains(&conn) {
            return false;
        }
        match g.machines.get(machine) {
            Some(m) => {
                send(&m.tx, msg);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn texts(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Outgoing>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(Outgoing::Text(t)) = rx.try_recv() {
            out.push(serde_json::from_str(&t).unwrap());
        }
        out
    }

    #[test]
    fn first_viewer_starts_streaming_and_last_stops_it() {
        let hub = Hub::default();
        let (mtx, mut mrx) = unbounded_channel();
        hub.machine_connected(7, 1, mtx);
        let (b1, mut b1rx) = unbounded_channel();
        let (b2, _b2rx) = unbounded_channel();
        hub.browser_connected(10, 1, b1);
        hub.browser_connected(11, 1, b2);

        hub.watch(10, "s1", 7, true);
        let m = texts(&mut mrx);
        assert_eq!(m[0]["type"], "watch");
        assert_eq!(m[0]["on"], true);
        assert_eq!(texts(&mut b1rx)[0]["live"], true);

        hub.watch(11, "s1", 7, true);
        hub.watch(10, "s1", 7, false);
        let _ = texts(&mut mrx);
        hub.watch(11, "s1", 7, false);
        let m = texts(&mut mrx);
        assert_eq!(m.last().unwrap()["on"], false);
    }

    #[test]
    fn output_goes_only_to_watchers_and_input_only_from_them() {
        let hub = Hub::default();
        let (mtx, mut mrx) = unbounded_channel();
        hub.machine_connected(7, 1, mtx);
        let (b1, mut b1rx) = unbounded_channel();
        let (b2, mut b2rx) = unbounded_channel();
        hub.browser_connected(10, 1, b1);
        hub.browser_connected(11, 1, b2);
        hub.watch(10, "s1", 7, true);
        let _ = (texts(&mut b1rx), texts(&mut mrx));

        hub.output(7, &json!({ "type": "output", "id": "s1", "data": "aGk=" }));
        assert_eq!(texts(&mut b1rx).len(), 1);
        assert!(texts(&mut b2rx).is_empty());
        // Another machine can't write into this session's viewers.
        hub.output(8, &json!({ "type": "output", "id": "s1", "data": "eA==" }));
        assert!(texts(&mut b1rx).is_empty());

        assert!(hub.input(10, "s1", "eA=="));
        assert!(!hub.input(11, "s1", "eA=="));
        assert_eq!(texts(&mut mrx).last().unwrap()["type"], "input");
    }

    #[test]
    fn a_stale_connection_doesnt_mark_the_machine_offline() {
        let hub = Hub::default();
        let (a, _ra) = unbounded_channel();
        let (b, _rb) = unbounded_channel();
        hub.machine_connected(7, 1, a);
        hub.machine_connected(7, 2, b);
        assert!(hub.machine_disconnected(7, 1).is_none());
        assert!(hub.machine_online(7));
        assert!(hub.machine_disconnected(7, 2).is_some());
        assert!(!hub.machine_online(7));
    }
}
