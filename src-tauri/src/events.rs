//! Pushes backend events to the UI. Event names are mirrored in `ui/src/lib/api/events.ts`.

use aether_core::ConnectionId;
use aether_ssh::{ForwardStatus, SessionEvents, SessionState, TransferEvents, TransferInfo};
use tauri::{AppHandle, Emitter};

use crate::dto::{ForwardsEvent, LatencyEvent, SessionStateEvent, TransferEvent};

pub struct AppEvents {
    pub app: AppHandle,
}

impl SessionEvents for AppEvents {
    fn session_state(&self, id: ConnectionId, state: &SessionState) {
        let _ = self.app.emit("session:state", SessionStateEvent { connection_id: id, state: state.clone() });
    }

    fn forwards_changed(&self, id: ConnectionId, forwards: &[ForwardStatus]) {
        let _ = self.app.emit("session:forwards", ForwardsEvent { connection_id: id, forwards: forwards.to_vec() });
    }

    fn latency(&self, id: ConnectionId, rtt_ms: u32) {
        let _ = self.app.emit("session:latency", LatencyEvent { connection_id: id, rtt_ms });
    }
}

impl TransferEvents for AppEvents {
    fn changed(&self, info: &TransferInfo) {
        let _ = self.app.emit("transfer:update", TransferEvent { transfer: info.clone() });
    }
}
